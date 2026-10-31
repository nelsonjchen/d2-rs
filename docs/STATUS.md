# D2 parity status

Evidence JSON cited below under `docs/*.json` was retired from the tree at
9b706ac1 and in a second pass on 2026-10-31 (which also removed the v0.9.0-era
TALA probes that produced it); `docs/RETIRED-EVIDENCE.md` lists every path and
how to read it back. Checkers now write reports under the gitignored
`oracle-cache/reports/`.
Oracle fixtures are regenerated from recipes (`docs/FIXTURES.md`).

## Oracle fixtures from recipes, 2026-10-31

All committed Go and JS oracle output is now regenerated from 559 recipes at
the pin (6a7b48378) into `oracle-cache/`: 695 artifacts, about 866 MB. Two
passes did this, first the fixtures over 256 KB and then the rest. Fixtures
that pinned unreachable internal states, duplicates and unread files were
retired or deleted; every disposition is in `docs/fixture-retirements/`. What
stays committed is hand-written: D2 seeds, request lists, test code and small
seed inputs under `tools/fixtures/seeds/`. The tracked tree is 3,137 files,
about 57 MB (it was about 1.66 GB).

**ARM verification** (d2rs-slim, c4a, `--goarch arm64`, at f98f6bac):
- All 559 recipes regenerate, and a second `--force` run gives byte-identical
  artifacts. Go's own nondeterminism is handled in the generators: temp
  paths, ports, dates and goroutine stacks are canonicalized, and PDFs whose
  image order follows Go map order record every variant Go produces.
- `--check` reports every recipe fresh; `scripts/test_regen_fixtures.py`
  passes.
- The default tier fails only tests that already failed at 9b706ac1:
  - `paper_pattern_vector_tile`, a multiply overflow at `scanline.rs:1959`;
  - the `d2-rs-isometric` lib, a stack overflow in `gif_loop` tests.

**Oracle tier:** 4103 passed, 68 failed, plus the isometric abort. These are
real drift at the pin, recorded as failing witnesses:
- **56 math:** `d2-rs-math` follows mathjax-go v0.1.0. Master pins
  v0.1.1-0.20261002175158-94d80186bfd7, which changes svg widths and heights,
  some errors and the default command count (1664 → 1657); in the nil-child
  requests all 148 now compile in Go. This includes the d2-rs-wasm nil-child
  test.
- **`pinned_go_ir_compiler`, case 50:**
  `a; steps: {one: caption; two: {b}; three: {c}}` now panics in Go's
  compiler (nil dereference in overlay → reserveVariableCopy).
- **CLI board-output manifest** (`named_wildcards_write_complete_source_board_trees`,
  both d2-rs-cli binaries): the Go CLI at the pin writes
  `.d2-board-output-manifest-v1.json` into multi-board output folders; d2-rs
  does not.
- **`positioned_unicode_callouts`:** Go's own pixels changed after v0.9.0
  (all 48 unicode rows), and d2-rs produces a third hash.
- **`svg_precision` mixed PNG:** master rounds the aspect matrix explicitly
  in `d2scene/aspect.go`; 80 rows differ in the last bit.
- **LaTeX admission:** `d2latex.ValidateInput` checks size before UTF-8 and
  reports a different message.
- **`ascii_target` outcomes:** master's `ValidateRenderTarget` rejects more
  input.
- **URL decode:** master's decode errors repeat their cause (`url_codec`,
  `urlenc`).
- **`public_request_unicode_decoding`:** master's ELK adapter writes
  `spacing.edgeEdgeBetweenLayers` instead of
  `elk.layered.spacing.edgeEdgeBetweenLayers`.
- **Already failing at 9b706ac1:** `paper_pattern_vector_tile` and the
  isometric abort.

**Outstanding:**
- Fix the drift above, starting with the mathjax-go port.
- Qualify x86-64 parity with `--goarch amd64` captures on an x86-64 host. Go
  fuses multiply-add only on arm64, and all evidence so far is arm64; see
  "Host architecture" in `docs/FIXTURES.md`.
- Two hard-coded assertions in `crates/d2-rs-wasm/tests/test-node.mjs`
  (around lines 613 and 719) fail at master.
- `request-unicode-source-v090` stores a JS `e.stack` with the host's node
  package path.

## WASM vs Go WASM (`d2js`), 2026-10-31

d2-rs's WASM package (`crates/d2-rs-wasm`, wasm-bindgen 0.2.128) was compared
with Go D2's own WASM build (`d2js` at 6a7b48378, `GOOS=js GOARCH=wasm`,
stamped with the same version string).

Setup:
- Node 22 on a c4a-standard-8 VM, both in-process, every measurement in a fresh process.
- 12 perf-bench cases × 3 engines, median of 3 after one warm-up.
- Harness: `scripts/wasm-vs-go/`.

At `508ce533`, **36/36 SVGs are byte-identical to Go's and every row is faster than Go's WASM**:

| Engine | median wall | geomean | total | median peak RSS |
|---|---:|---:|---:|---:|
| Dagre | 0.30x | 0.29x | 0.31x | 0.33x |
| ELK | 0.29x | 0.28x | 0.33x | 0.32x |
| TALA | 0.40x | 0.40x | 0.27x | 0.61x |

| | Go `d2js` | d2-rs |
|---|---:|---:|
| Module size (raw / gzip -9) | 60.0 / 15.3 MB | 27.2 / 9.2 MB |
| Startup (instantiate + init) | ~405 ms | ~43 ms |
| TALA `tpmjs_architecture` | 58.6 s | 15.3 s |
| TALA `arrowhead_scaling` | 29.6 s | 8.2 s |

TALA's WASM lead is bigger than its native one: Go D2 loses goroutine
parallelism in WASM, and d2-rs's single-thread CPU wins carry over.

Before `508ce533`, `grid_large_checkered` was 1.03-1.06x on all engines. The
WASM compile response was built as a `serde_json::Value` tree and then
serialized. Serializing borrowed data directly, with the same key order and
byte-identical JSON, took that board's compile from 103 to 61.5 ms (Go: 71 ms).

**Outstanding:** `crates/d2-rs-wasm/scripts/test-node.mjs` fails identically
with and without `508ce533`:
- `dagre/empty.renderOptions` has an extra `isometric: false`.
- "legacy getCompletions did not interpret Go UTF-16 columns".

Not measured: browser engines, and a faster `wasm32` allocator than dlmalloc
(estimated 5-8% of compile).

## 2026-10-31: faster and smaller than Go on every perf-bench case

The full perf-bench at `e1f1e13b` ran on d2rs-dev, idle, with `--repeat 5`.
Outputs were byte-identical on 36/36 rows. No row is at or above 1.0x Go in
wall time or in peak RSS.

| Engine | median wall | total wall | worst wall | median RSS | worst RSS |
|---|---:|---:|---:|---:|---:|
| Dagre | 0.34x | 0.52x | <0.9x | 0.53x | <0.9x |
| ELK | 0.34x | 0.46x | <0.9x | 0.53x | <0.9x |
| TALA | 0.71x | 0.88x | 0.92x (tpmjs 3.08 vs 3.35 s) | 0.59x | 0.95x (tpmjs 122 vs 129 MB) |

| Commit | Change | tpmjs peak RSS |
|---|---|---|
| 44b958b5 | 8-byte route queue entries | 151 → 144 MB |
| e3ed1d5f | streamed segments; axis buckets moved at freeze | 145 → 143 MB |
| 754e6e33 | TALA routing release hook; the CLI runs `mi_collect` after each routing pass | 142 → 127 MB |
| e1f1e13b | CLI sets the mimalloc purge delay to 2 ms (default 10 ms) | 128 → 117 MB (Go 128 in the same run) |

What decided these last steps:
- Peak-moment attribution showed tpmjs's live data peaking at about 113 MB. The remainder was freed routing memory that mimalloc had not yet purged.
- A purge delay of 0 ms reached 112 MB but cost about 5% wall time. 2 ms cost about 1%.
- Purging the routing workers' heaps as well made no measurable difference, so it was not committed.

## Memory 2026-10-31: TALA tpmjs within 1.2x of Go

The full perf-bench at `9c429231` ran on d2rs-dev, idle, with `--repeat 5`.
Outputs were byte-identical on 36/36 rows.

| Engine | median wall | total wall | worst wall | median RSS | worst RSS |
|---|---:|---:|---:|---:|---:|
| Dagre | 0.34x | 0.51x | <0.8x | 0.53x | <1x |
| ELK | 0.33x | 0.47x | <0.8x | 0.54x | <1x |
| TALA | 0.71x | 0.86x | 0.90x (tpmjs) | 0.60x | 1.19x (tpmjs 157 vs 132 MB) |

tpmjs peak RSS went 514 → 157 MB over this series. arrowhead is now
0.92x of Go's RSS.

| Commit | Change | tpmjs peak RSS |
|---|---|---|
| f8eb59f2, c453dbff, 2b5dd358, 835a6513 | frozen OVGs drop build tables; compact search slots and queue records; the OVG build is moved, not cloned; compact static edges | 249 → 207 MB |
| 6a787f2f | one flat 16-byte adjacency table; u32 OVG slots; slot-indexed static edges | 206 → 186 MB |
| 9c429231 | at most two seeds of a race route at once (output-exact: every seed still runs; winner by score then index) | 186 → 148 MB, wall +11.6% |

Measured and not taken:
- A routing gate of 1 seed: 122 MB, but +24% wall, which puts tpmjs at 1.01x Go.
- Geometry-only pre-`direct()` snapshots: the mirroring helper needs full nodes.
- A per-root OVG arena: node identity is shared across merged builds, so it isn't exact as a local patch.

## Performance and memory 2026-10-30 (night): faster and leaner than Go

The full perf-bench at `5bfab943` ran on d2rs-dev, idle, with `--repeat 5`.
Outputs were byte-identical on 36/36 rows.

| Engine | median wall | total wall | worst wall | median peak RSS |
|---|---:|---:|---:|---:|
| Dagre | 0.33x | 0.51x | <0.75x | 0.53x |
| ELK | 0.33x | 0.46x | <0.75x | 0.53x |
| TALA | 0.70x | 0.82x | 0.90x (`spyre_encoder`) | 0.62x |

TALA peak RSS outliers: `tpmjs_architecture` is 258 vs 133 MB (1.94x, down
from 3.96x), and `arrowhead_scaling` is 54 vs 53 MB (1.01x, down from 3.12x).

| Commit | Change | Effect |
|---|---|---|
| 88f533d4 | cache-line `ArenaNode`/`ArenaEdge` layout, stable sizeless sweeps, inlined overlap scan, routing guard fast paths | about -3% on every TALA target |
| 457fd059 | compile once / validate once in the CLI, sorted font tables over static bytes, lazy markdown regex, decoded-once flag table | grid boards -22% on all engines, spyre 106 → 101ms |
| 5bfab943 | `PR_SET_THP_DISABLE` on Linux; pooled per-thread route search state; one static edge table per OVG shared by all flavors | tpmjs RSS 478 → 251 MB, spyre 67 → 33 MB; +2-3% wall on the large TALA boards |

**Update at `835a6513`** (full perf-bench, dev, `--repeat 5`, 36/36 identical):
- Commits:
  - f8eb59f2: frozen OVGs drop build tables.
  - c453dbff: compact search slots and queue records, lean OVG slots.
  - 2b5dd358: the OVG build is moved into its canonical view, not cloned.
  - 835a6513: compact static edge records.
- Wall time:
  - TALA: median 0.70x, total 0.81x; worst `spyre_encoder` 0.89x.
  - Dagre: median 0.34x, total 0.51x.
  - ELK: median 0.33x, total 0.46x.
- Peak RSS:
  - Medians: TALA 0.61x, Dagre 0.53x, ELK 0.53x.
  - `tpmjs_architecture`: 214 vs 126 MB (1.69x).
  - `arrowhead_scaling`: 51 vs 53 MB (0.96x).
- tpmjs's remaining memory is mostly the routing object graph: one `Arc<GoCell<PortNode>>` per OVG node, with incident-edge headers, for every flavor of three concurrent seeds. Removing it means replacing pointer identity in routing, a structural change rather than an exact local refactor.

What was measured before choosing the THP setting:
- With THP set to `always`, the kernel backs mimalloc's arenas with 2 MiB pages that layouts touch only sparsely.
- THP off saved 28-55% RSS for 2-4% wall time.
- Disabling arena eager commit saved a third as much for about 2.4%.
- Immediate purge cost about 8% wall time.
- Changing the rayon pool size made no difference.

## Performance 2026-10-30 (evening): every perf-bench case at or below Go

The full perf-bench at `81675c36` ran on d2rs-dev, idle, with `--repeat 5`.
Outputs were byte-identical on 36/36 rows.

| Engine | median Rust/Go wall | geomean | total | worst case |
|---|---:|---:|---:|---|
| Dagre | 0.36x | 0.44x | 0.55x | 0.86x (`grid_large_checkered`) |
| ELK | 0.35x | 0.43x | 0.51x | 0.87x (`grid_large_checkered`) |
| TALA | 0.75x | 0.69x | 0.85x | 0.99x (`spyre_encoder`) |

Quiet host (c4a-standard-8, median of 21, both orders), Rust vs Go:

| Case | Rust | Go | Rust/Go |
|---|---:|---:|---:|
| `real_world/tpmjs_architecture` | 2.87s | 3.35s | 0.86x |
| `stable/arrowhead_scaling` | 1.60s | 1.85s | 0.87x |
| `stable/giant_markdown_test` | 0.148s | 0.190s | 0.78x |
| `real_world/spyre_encoder` | 0.110s | 0.109s | 1.00x |

`spyre_encoder` is at parity with no margin. A critical-path analysis found its
whole remaining gap in the slowest seed's placement: about 1.5x Go's
placement CPU per seed. Before the race (13.6 vs about 27 ms under perf) and
after it (about 10.4 vs 10.5 ms), Rust is already at or ahead of Go. Seed count,
concurrency and fan-out match Go.

| Commit | Change | Effect (dev unless noted) |
|---|---|---|
| 112a5236 | sized generation `seen` cells and spatial-index occupancy become hash collections (membership only; candidate order unchanged) | quiet: giant_markdown 0.225 → 0.169s; spyre 0.128 → 0.120s |
| 6b8924fc | lazy ancestry overflow sets, slice-only obstruction sets, single validated frame per scorer pass, reused full-scan member header and exception slice | tpmjs -1.0%, giant_markdown 171 → 159ms, spyre 122 → 118ms |
| 81675c36 | in-place refill of uniquely owned edge/near storage on `clone_from`, shared descendant walk, presized moved sets, lockless routing-cost mirror, owner-frame projection skip | tpmjs -1.7%, giant_markdown 160 → 151ms, spyre 119 → 113ms |

A grid reroute skip and a cached CPU count (bt36) measured neutral and were
not committed.

**Update at `2fd8be50`:**
- Change: memoized sizeless-scorer frame validation, lazy replacement chains, paired endpoint reads in the overlap scan.
- Effect on dev: tpmjs -1.1%, spyre 111 → 109ms.
- Full perf-bench (dev, `--repeat 5`):
  - TALA: median 0.74x, total 0.84x.
  - Dagre: median 0.35x, total 0.54x.
  - ELK: median 0.35x, total 0.51x.
  - Byte-identical on 36/36 rows.
  - The worst case is `spyre_encoder` at 0.97x; every other case is at most 0.88x.
- Quiet host (both orders): spyre 0.107s vs Go 0.109s (0.98x), giant_markdown 0.79x, tpmjs 0.85x, arrowhead 0.85x.
- Flat rollback snapshots with pooled translation sets (bt38) measured within noise and were not committed.

## Performance series 2026-10-29/30: TALA large boards beat Go

The full perf-bench at `65321ee2` ran on d2rs-dev (c4a-standard-16, idle) with
`--repeat 5`. Outputs were byte-identical on 12/12 cases per engine.

| Engine | median Rust/Go wall | geomean | total | CPU total | RSS |
|---|---:|---:|---:|---:|---:|
| Dagre | 0.36x | 0.43x | 0.54x | 0.41x | 0.93x |
| ELK | 0.35x | 0.43x | 0.51x | 0.39x | 0.95x |
| TALA | 0.80x | 0.74x | 0.89x | 0.76x | 1.52x |

Quiet-host timing (c4a-standard-8, interleaved, median of 15, both orders):

| Case | Engine | 8a4cbaa7 | 65321ee2 | Go | Rust/Go |
|---|---|---:|---:|---:|---:|
| `real_world/tpmjs_architecture` | TALA | 3.41s | 2.97-2.99s | 3.35s | **0.89x** |
| `stable/arrowhead_scaling` | TALA | 1.74s | 1.654s | 1.847s | **0.90x** |

**Failing at 65321ee2 (both since fixed; see the section above):**
- TALA `stable/giant_markdown_test`: 0.227s vs 0.193s (1.18x).
- TALA `real_world/spyre_encoder`: 0.131s vs 0.114s (1.15x).

Both were already this slow at 8a4cbaa7 (quiet: 0.220s and 0.126s against Go's
0.189s and 0.109s), so this series did not regress them. They are the next
targets.

Every commit kept the e2e scoreboard at **933 SVG + 11 expected errors** with
identical output hashes. Each passed the TALA tests three times, the front-end
crate tests and the `diagnostic-traces` check. Each change is an exact refactor
from a static Rust/Go comparison of a profile hotspot. Five parallel
patch-only scout agents proposed them; each batch was validated on dev and
then timed on the quiet host.

| Commit | Change | tpmjs (quiet) |
|---|---|---|
| 2731ed55, 8a4cbaa7 | in-place reverse DFS, aggregate-free sync skips, cached carriers; lighter rotation rollback | 3.60 → 3.48s |
| 1df10dc0 | routing: hole-based heap sifts, per-node search hoists, cached Done scope, lazy interaction checks, moved OVG incident edges | 3.405 → 3.319s |
| 40f73625 | packing/align: dense overlap-pair bitset with prefilters first, cached ancestry chains under stable packing topology (exact guard step counts), dense commit flags | 3.413 → 3.261s (measured alone) |
| b20764d8 | placement: replayed owner ancestry between polls, fixed-origin skips, cached symmetry matches | 3.315 → 3.214s (measured alone) |
| 65321ee2 | canonical search-node table, dense static edge index, per-search cluster sets, Arc-shared visibility, field-wise `ArenaNode` clone, buffer-reusing rollbacks and restores | 3.03-3.07 → 2.97-2.99s |

## Performance series 2026-10-28: every non-TALA target beats Go

Quiet-host timing at `5eafdbfd`, on a fresh idle c4a-standard-8 spot VM.
Interleaved runs (7 for TALA, 21 for grid, 15 for the rest) compare the
pinned Go oracle with the d2-rs release CLI. Every pair produced
byte-identical SVG hashes.

| Case | Engine | Go | d2-rs | Rust/Go |
|---|---|---:|---:|---:|
| `real_world/tpmjs_architecture` | TALA | 3.365s | 5.195s | **1.54x** (1.9x at b2c5e7f0) |
| `stable/arrowhead_scaling` | TALA | 1.850s | 2.512s | **1.36x** (1.78x at b2c5e7f0) |
| `stable/grid_large_checkered` | Dagre / ELK / TALA | 0.024-0.025s | 0.022s | 0.89x / 0.89x / 0.90x (2.7x at b2c5e7f0) |
| `real_world/tpmjs_architecture` | Dagre / ELK | 0.130 / 0.051s | 0.090 / 0.033s | 0.69x / 0.64x |
| `stable/arrowhead_scaling` | Dagre / ELK | 0.077 / 0.044s | 0.047 / 0.027s | 0.61x / 0.60x |

TALA is the only engine still above 1.0x on the perf-bench targets.

Every commit in the series has these properties:
- The e2e scoreboard stays at **933 SVG + 11 expected errors**.
- Output hashes are identical.
- Validation used the crate test suites (TALA three times, to catch parallel races) and the
  `diagnostic-traces` check.
- Lane changes were validated from run-exit snapshots and applied as increments over HEAD.

- **Dagre:**
  - 4dbea00b: tree-edge index in network simplex.
  - 08a67747: dense cut values, tree adjacency, Brandes-Koepf predecessors and a label-size
    snapshot; x0.70 on both targets.
- **Front end and render (grid 0.071s → 0.022s):**
  - a241759f, 87a3f5b4: board planning skips the render-only projections.
  - 54e7d003, 73cde9f8: class lists and resolved fields are shared, and the board plan reuses
    the document's compile.
  - f2056126: render lowering skips the inspection JSON; metadata comes from the layout compile.
  - 15a74ba2, 6356285f: indexes for canonical children and in-place scalar reads.
  - fc79eff3: plain-key fast path; style-map validation is replayed.
  - 862d88c5: late validation reuses resolved class names.
  - 5eafdbfd: typed records go to `TargetDiagram` directly, with no JSON round trip.
- **TALA:**
  - Sizeless optimizer: ea96eb68 publishes owner geometry once per pass.
  - Sized optimizer: ccaec5c8 publishes, aligns and scores flow directly on the owner, as Go
    mutates one graph.
  - Routing: a63e5c0c visits only real label boxes.
  - Swap:
    - 5b478518, 7db157ad: global scoring is pruned when the local cost already decides.
    - 8c6381cc: the trial baseline is reused across a node's candidates.
  - Transactions and alignment:
    - bb3e8900: overlap checks only for pairs that touch dirty nodes (`transaction.go:900`).
    - 19a61216: failed alignment and transpose attempts stop before refit and validation.
  - Container refit: 0262bfe7 reads `Graph.Nodes` once.
  - Bin-pack: 266b1cb5 reads transaction nodes directly.
  - Lookups and handles:
    - 5f036a20: dense vessel bindings.
    - e9c9701c: connected-set inverse links.
    - d5364619: ownership test per candidate; borrowed points.
    - 9d3078e9: single-header detach.
    - 8da4f472: borrowed group entries.
    - 284415bd: projection refresh without a lookup table.

Remaining TALA profile at fc79eff3, inclusive:

| Area | tpmjs | arrowhead |
|---|---:|---:|
| Sized pass | 16% | 22% |
| Routing | 18% | |
| Gap normalization (`refit_containers` 7.6% / 13.3%) | 15% | 23% |
| Bin-pack | 9% | 7% |

Arc refcount atomics are about 10% of self time, spread across every stage. They are the
largest single self cost.

## TALA shared-pointer series (2026-10-27, d2rs-dev c4a-standard-8)

Target: perf-bench TALA, profiled on `real_world/tpmjs_architecture`.
Every step keeps the e2e scoreboard at **933 SVG + 11 expected errors**
and identical output hashes. The env-gated equivalence checks below were
temporary validation builds, not committed code.

| Commit | Change | tpmjs TALA |
|---|---|---|
| d05b0825 | mimalloc global allocator (CLI) | baseline 10.10s |
| b3451dba | transpose rollback journals external geometry; full copies only for child entries the refit republishes whole | 10.10 → 9.45s (x0.935) |
| 01fc1d15 | exact-zero `translate_node_with_children` returns early (the sized mirror re-applies every node); fixed-origin prefilter before the O(n²) `parent()` walk in commit validation | 9.34 → 8.62s (x0.923) |
| db2612c8 | `GoCell` replaces `RwLock` for shared Go-pointer fields; `checked-cells` (and tests) track borrows and panic on conflict | 8.57 → 7.74s (x0.903) |
| "drop two whole-graph trial clones" | bin-pack trials stop cloning the graph for trace-only IDs; equidistance trials roll back via the trial geometry journal instead of a whole-graph clone | 7.79 → 7.53s (x0.967) |

Go takes ~3.4s on the same host, so this board moved from ~2.9x to ~2.2x.

perf-bench at db2612c8 (12 cases, outputs identical 12/12 per engine;
Rust/Go wall time):

| Engine | Median | Geomean | Total |
|---|---|---|---|
| TALA | 1.05x | 1.24x | 2.17x |
| Dagre | 0.45x | 0.75x | 1.33x |
| ELK | 0.43x | 0.70x | 1.09x |

TALA RSS median is 1.45x Go. The large boards still fail the 1.0x gate on
every engine.

perf-bench at ceadcc1e, after the front-end and Dagre series (9be45f98
class-maps memo, 707e879a ASCII lowercase, 45ae75b5 one config compile per
render, 49af2c03/0c8b3db7/e6f6738d/ceadcc1e network simplex and BkModel):

| Engine | Median | Geomean | Total |
|---|---|---|---|
| TALA | 1.03x | 1.13x | 2.09x |
| Dagre | 0.42x | 0.62x | 0.91x |
| ELK | 0.40x | 0.60x | 0.81x |

Still failing: `grid_large_checkered` on every engine (~2.7x, front end), dagre
`tpmjs_architecture`/`arrowhead_scaling` (~1.15x), and TALA's large boards
(tpmjs 2.17x, arrowhead 2.21x, giant_markdown 1.57x).

TALA after the follow-up series (8ecefd26 in-place incident slots, dbdcb773
and 39986a5a bin-pack commit bounds/primary list, 074487bd route-guard serial
fast path): tpmjs 6.84s (~2.0x Go 3.44s), arrowhead 3.55s (~1.9x Go 1.88s).
Two ideas measured neutral and were dropped: parallel EdgeLength scoring
inside seed workers (x0.999/x1.017) and the `--target=*` plan-lowering
shortcut (x1.00).

perf on both implementations (tpmjs): Go spends ~62% of its CPU samples in
`runAllStages` (nodePlacement 22%, gapNormalization 12%, swapNodes 12%,
edgeRouting 5%); the other ~38% is GC/runtime on spare cores. Rust has no
GC share, so algorithm CPU is ~2.5x Go. The remaining Rust profile is flat:
Arc refcounts, group-membership lookups and commit validation each sit near
1%, so further hotspot fixes give 2-5% each. The adapter layers that emulate
Go's shared pointers (owner-session projection refreshes, the sized movement
mirror, per-stage topology preflight, trial snapshots) are the larger
structural target.

- Transpose: a check compared journal rollback with the old whole-node
  clone after every rotation. It found two whole-node republish paths in the
  refit (local containers into aliasing children, external containers into
  children and aggregates). Both are now journaled. After that, no mismatch on
  tpmjs or the 308 TALA boards.
- Zero moves: a check ran the full walk on each zero move and compared every
  node, projection, owner-point and vessel coordinate bitwise. Nothing changed.
- GoCell: Go uses plain pointers here. `GraphCostCache`, which is written
  concurrently by design, keeps its lock. The checked build caught one real
  conflict: empty-window writes on the process-wide zero owner-point backing.
  It surfaced only when unit tests ran in parallel. Those writes now return
  before borrowing. With the fix, the checked build passes every e2e board and
  three parallel test runs.
- An earlier no-lock bound measured x0.905. Remaining profile: Arc refcount
  atomics ~10% self, diffuse across dozens of header accessors (none above
  0.8%); whole-graph clones ~5% (packing transaction capture, equidistance,
  bin-pack); routing ~13%.
- Equidistance: a check compared the whole graph's `Debug` after the
  geometry rollback with the old clone restore after every trial. No mismatch
  on tpmjs or any TALA board.
- Measurement note: two runs in this series were first timed on a VM checkout
  that lacked db2612c8, against a db2612c8 baseline. They were retimed on the
  synced checkout; only the retimed numbers are recorded.
- Neutral, reverted: a lazy tala-id index in the translate fixpoint (x1.002).

## Lane I67 — TALA performance continuation (2026-10-26, VM)

The release e2e baseline remains **933 SVG + 11 expected errors**, with
all 966 classifications unchanged from I66 (fixed/regressed **0/0**).
I66's flat Spyre profile identifies atomic and node-slice read costs, but
does not attribute them to a specific safe call path. The node-slice iterator
explicitly reads live backing on each step; batching it without proving the
caller's mutation behavior would change its contract. No implementation or
performance improvement is claimed. A symbolized call-graph profile is the
next step. The TALA raster corpus is not mirrored on this VM.

The native compatibility goal remains **active and incomplete**. Since
2026-10-24 the oracle is upstream Go D2 **master at
`6a7b4837838a3907604a1d9eec57ee98b6f3cd01`** (2026-10-11, 107 commits after
v0.9.0, same go1.27.0 toolchain), including bundled in-process TALA. The
v0.9.0 target is closed: its final d2-rs checkpoint is the tag
`d2-v0.9.0-final` (a3f19393). External layout plugins are excluded (master
removed `d2plugin`). Retained corpus matches are evidence for those cases, not
a percentage or proof of full D2 parity.

## Lane L88 — validation performance (2026-10-26, VM)

`late_validation::Board::compile_map` now computes each unquoted field name's
Go-lowercased bytes once for its keyword checks, retaining Unicode semantics
and lookup order. The pinned compiler's keyword handling is in
`d2compiler/compile.go:453,655,1145`. A symbolized Spyre Dagre sample identified
lowercase construction in this path; the post-change single-case sample still
shows atomic reference counts/locks and `graph_field` among the hot paths.
`Shared<T>` is public and replacing its `Arc<RwLock<T>>` would remove Send/Sync
auto traits; that API change is not qualified.

E2e remains **933 SVG + 11 expected errors**, all 966 classifications unchanged
(fixed/regressed **0/0**). Core and language library tests pass **72 + 71**.
Five-repeat VM-noisy Rust/Go CPU ratios before → after with bundling disabled:
Dagre **1.06 → 1.04**, ELK **1.00 → 0.92**, TALA **1.72 → 1.71**; all
36 rows match complete Go SVG bytes. Concurrent activity and host load prevent
attributing those ratios to this change. Language goldens and master compiler
captures are absent from this VM. See `.reference/codex-runs/report-lang88.md`.

## Lane D VM grid investigation (2026-10-26)

Lane D's sixth run scored the existing candidate release binary: **933 SVG /
11 expected errors**, all 966 classifications unchanged against
`dagre5-before.json` (fixed/regressed **0/0**, `dagre6-start.json`). The
source-matched baseline build was blocked on the shared Cargo lock. A bounded
five-repeat candidate benchmark using `--bundle=false` on both sides was
stopped incomplete; no paired timing or new profile is claimed. The candidate
remains unapplied. See `report-dagre6.md`.

Lane D's fifth run built the saved grid candidate and verified the full e2e
guard: **966 unchanged classifications**, **933 SVG / 11 expected errors**,
fixed/regressed **0/0** (`dagre5-before.json`, `dagre5-after.json`). The
layout library suite passed **104 tests, 4 ignored**. The candidate is reversed:
the five-repeat baseline benchmark had three failed `stable/label_positions`
rows because the sandbox could not fetch an external icon, and the candidate
benchmark was stopped. Its baseline binary predates other lanes' current edits,
so it also cannot isolate this change. No candidate speedup or complete perf
parity is claimed. `target/release/d2-rs` is the candidate build and does not
reflect the reverted grid source; rebuild before using it as a source-matched
baseline. Details and continuation are in `report-dagre5.md`.

Lane D's fourth run rechecked the release baseline: **933 SVG / 11 expected
errors**, all 966 classifications unchanged against `dagre3-before.json`
(fixed/regressed **0/0**; `dagre4-before.json`). The saved grid candidate was
temporarily applied, but the shared release lock remained held and the
five-repeat baseline benchmark did not finish. Both jobs were stopped and the
candidate reversed; no candidate performance or regression result is claimed.
The lock owner was outside the sandbox's visible process namespace, and a disk
scan observed another active release build. See `report-dagre4.md`.

Lane D's third run measured the release baseline again: **966 unchanged
classifications**, **933 SVG / 11 expected errors**, fixed/regressed **0/0**
against `dagre2-before.json` (`dagre3-before.json`). The existing grid cache
candidate was temporarily applied, but the shared release build remained
blocked on Cargo's target lock and the five-repeat baseline benchmark did not
finish; both jobs were stopped and the patch reversed. No implementation,
candidate delta, timing improvement, or new profile is claimed. The saved patch
and pinned source mapping remain in `report-dagre1.md`; raster corpora are
not mirrored on this VM. See `report-dagre3.md` for the continuation.

Lane D's second run rechecked the release baseline: all **966 classifications**
remain unchanged from `dagre1-before.json` (**933 SVG / 11 expected errors**;
fixed 0, regressed 0), recorded in `.reference/codex-runs/dagre2-before.json`.
The grid candidate remains unapplied: the shared release target lock blocked
the build and the five-repeat baseline benchmark did not finish before both
jobs were stopped. No candidate performance or regression result is claimed.
Pinned `grid_search.go:24–86` also includes an integral-prefix fast path not
present in the saved candidate. Raster and other mirrored corpora are absent
on this VM. See `.reference/codex-runs/report-dagre2.md`.

The release baseline remains **933 SVG / 11 expected errors** across the 966
inventoried e2e rows (`.reference/codex-runs/dagre1-before.json`). A bounded
row-measurement cache for the profiled grid division search was investigated
against pinned `d2layouts/d2grid/grid_search.go:12–53,88–99` and
`layout.go:610–698`; it preserves the existing floating-point sum and division
order. The candidate passed the layout library suite (**104 passed, 4 ignored**),
including a fractional-row and collision bit check. It is **not applied**:
the shared release build lock did not become available and the five-repeat
baseline benchmark did not finish, so no candidate scoreboard delta or speedup
is claimed. The candidate patch and continuation instructions are in
`.reference/codex-runs/report-dagre1.md`. Raster and other corpora are absent
on this VM.

## Lane L87 — front-end validation performance (2026-10-26, VM)

Arrowhead validation now uses borrowed unquoted property lookup and avoids
allocating lowercase strings for ASCII keyword comparisons, retaining Go's
Unicode lowercase fallback and the original field order. It reuses the
previously computed reserved-field decision. Mapping: pinned `d2compiler/compile.go`
`compileMap`/`compileField` and `d2ir/d2ir.go` `GetField`.

Full e2e: **933 SVG + 11 expected errors**, all 966 classifications unchanged;
delta **0 fixed / 0 regressed**. Core and language library tests: **72 + 71
passed**. Full integration test compilation was interrupted. The language
golden and master compiler corpora are not mirrored on this VM.

Five-repeat VM-noisy CPU ratios before → after, using `--bundle=false` on
both sides: Dagre **0.93 → 0.91**, ELK **0.94 → 0.85**, TALA **1.53 → 1.48**;
all **36/36** outputs match Go on every measured repetition. These are shared
host measurements, not isolated speedup claims. The ordinary benchmark had
33 exact rows and three failures on both sides because the sandbox could not
fetch a remote icon. A fresh symbol profile remains outstanding: the release
binary is stripped. See `.reference/codex-runs/report-lang87.md`.

## Retarget to D2 master 6a7b48378 (2026-10-24; latest)

Pins: `reference-manifest.json` (`d2.commit`, `d2.oracle`
`.reference/bin/d2-master-6a7b4837-go1.27`, `d2.e2e_cases`
`.reference/e2e-cases-6a7b4837`, trace checkout `../d2-trace` = pin +
`tools/patches/d2-e2e-case-dump.patch`). Scripts read them through
`scripts/references.py`. The oracle keeps the v0.9.0 version stamp (unstamped
master reports `v0.8.1-HEAD`). Case dump: same 328 e2e cases, identical
scripts.

First measurement against master, then after this round's fixes:

| Scoreboard | v0.9.0 final | master, before | master, now |
|---|---|---|---|
| e2e SVG | 933 | 926 | **933** |
| raster D/E/T | 317/313/313 | 314/310/308 | **317/313/313** |
| CLI contract | 58/70 | 46/70 | **58/70** (51 goldens, +5 manifests) |
| cli-diff | 78/80 | 69/80 | **119/119** (explicit shared log/PDF clock; run 66) |
| flag matrix | 302/304 | 302/304 | **304/304** |
| golden tiers | 38/12/6/19 | 38/12/6/19 | 38/12/6/19 |
| inline | 35/28/40 | 35/28/40 | 35/28/40 |
| language | unchanged | unchanged | unchanged |
| Isometric native whole-board SVG | — | 0/870 | **870/870** |

Run 52 additionally qualifies **67/70 complete CLI artifacts** under the
explicit shared-UTC-clock / release-PPTX profile (63 before the PDF background
fix). One upstream skip and two unsupported injected writers remain non-passing.
The ordinary source-clock CLI tier in the table remains **58/70**, with all
**67 applicable assertions** passing and **51 golden matches**.

Baselines: `.reference/scoreboard-master-6a7b4837/`. TALA raster: the
harness timed out `real_world/tpmjs_architecture` and `stable/arrowhead_scaling`
under 8-way load (standalone 32 s / 54 s against Go's 5 s / 2 s; limit 180 s);
both match pixel-exactly when compared with the same gate. cli-diff's
formatter failure is fixed in run 66; explicit shared clocks qualify all
119 invocations. Ordinary clocks can still differ in timestamp bytes.

Fixes (all from master source):
- d2svg `lib/svg/markup.go` ScopedID: border-mask ids encode unsafe shape ids
  as `d2e-<base32>` (`stable/grid_outside_labels`, 3 engines);
  `table.go` borderRadiusClipPathID: class/table header `clip-path` uses the
  clipPath's encoded id (`stable/class_and_sqlTable_border_radius`, 3).
- TALA b34d2e936 PreserveCompoundRoutes/refineSeedResult
  (`crates/d2-tala-rs/src/internal/engine/compound_routes.rs`;
  `stable/elk_shim` TALA).
- d2cli `board_output.go`/`board_output_publish.go`
  (`crates/d2-rs-cli/src/board_output.rs`): multiboard output is staged and
  published transactionally with `.d2-board-output-manifest-v1.json`;
  unchanged stale generated files retire, user files survive; unsafe or
  reserved board names become `_d2_<sha256>`; colliding outputs are rejected;
  an empty document with child boards publishes only its manifest; watch
  renders publish the selected subtree the same way. Board links on
  connections are relinked too, with `url.PathEscape`d components (`main.go`
  relink/boardOutputLink). Raster multiboard cases 9 → 0 failures.
- d2cli `watch.go`: a per-process 32-byte access token (`?token=` exchanged
  for an HttpOnly `d2-watch-<port>` cookie, 303 redirect; 403 otherwise).
- `help.go`: built-in layout wording and layoutNotFound text.
- d2elklayout `EdgeEdgeSpacing`: `--elk-edgeEdgeBetweenLayers` (default 50)
  sent as `spacing.edgeEdgeBetweenLayers` at the root and every container
  (output byte-identical to Go at 10 and 120).
Harness: the raster gate and CLI scoreboard verify each side's board manifest
against its own files and compare the owned inventory (PNG encoder bytes are
not a contract); CLI manifest goldens apply the existing SVG version
allowance before hashing (tests in `scripts/test_render_artifact_gate.py`).

Done on master: `--allow-private-network`/`$D2_ALLOW_PRIVATE_NETWORK`
(lib/netpolicy public-only policy for remote images: local hostnames and
non-public answers rejected per connection, literal and redirect targets
checked, proxies disabled) and the imgbundler rewrite onto imageasset with
the localfile Unrestricted policy (SVG bundling and raster preflight share
one resolver; `failed to bundle image(s): imageasset: ...` text). Evidence:
39 new `scripts/cli-diff-scoreboard.py` image-asset cases, all matching Go
(loopback server, no external network). Outstanding on master, in
board-count order: `--isometric` (d2isometric: **870/870** native renderer
SVGs; CLI SVG/PNG/PDF/PPTX and help are wired; the paged corpus is **24/24**
and watch source/navigation is **20/20**; native GIF/timeline APIs are implemented,
with CLI GIF wiring qualified on 16 finite subprocess cases in run 65), and master's new language inline tests (Lane L11
matches 70/70 public compiler calls, 14/14 glob IR calls, 17/17 variable
IR calls, 11/11 edge IR calls, 65/65 cached-AST clone calls, 13/13 Markdown substitution calls and two class-index contracts covering 8,197 lookups; further
internal/context API adapters remain open; graph materialization now matches seven
original calls across four complete subtests).




### TALA hash maps for lookup-only state (2026-10-27)

TALA spends its time in the same phases as Go, in about the same shares; Go's
own top symbol is `runtime.mapaccess2_fast64`, while the port keeps ~2,000
BTreeMap/BTreeSet uses. Converting the lookup-only ones on the hottest paths
to `IdHashMap`/`IdHashSet` (an FxHash-style integer hasher in
`internal/mod.rs`) is the most productive change so far. Iterated maps stay
ordered, or are sorted where iterated, so outputs cannot change.

Clean bench VM, summed Rust/Go wall over the 36 rows (all byte-identical):

| Commit | Change | TALA |
|---|---|---:|
| 40f78c87 | before this series | 3.28 |
| ae4bf615 | lock-free group vessels, borrowed edge reads, pruned validation pairs | - |
| f49540a7 | hashed validation geometry, move sets, node_groups | - |
| b05ff899 | hashed owner points and move set; validation dirty flags and overlap set | - |
| 08881f23 | lazy connected-set index; hashed projection geometry | **2.82** |

`tpmjs_architecture` went from 3.72x to 3.09x and `arrowhead_scaling` from
2.79x to 2.57x (interleaved medians). TALA geomean is 1.37x; Dagre and ELK
are unchanged at 1.54x / 1.28x summed and 0.82x / 0.77x geomean.

Tried and rejected: inline LSE atomics (no change); `Rc<RefCell>` for
`GraphEdges` (the layout graph is shared across seed, scoring and routing
threads, so TALA's cells must stay `Sync`); reordering the fixed-origin scan
in `transaction_container_bad_state` (Go's `container()` can nil-panic first,
so the scan order is observable).

### Lock removal and front-end indexing (2026-10-26/27)

Summed Rust/Go wall on the clean bench VM (t2a-standard-4, 12 cases/engine,
5 interleaved repeats, 36/36 rows byte-identical and e2e at 933 SVG + 11
errors after every step):

| Commit | Change | Dagre | ELK | TALA |
|---|---|---:|---:|---:|
| e4062566 | baseline (grid row cache) | 1.83 | 1.65 | 3.77 |
| e5a90462 | AST `Shared`: `Rc<RefCell>` instead of `Arc<RwLock>` (nothing needs `Send`) | 1.69 | 1.47 | 3.71 |
| a4d49c68 | TALA `SharedOwnerPoint` as relaxed f64 bit patterns | - | - | 3.54 |
| 5c719ce2 | L93: late-validation children indexed by lowercased ID | 1.55 | 1.30 | 3.49 |
| aa04fbbe | I76: no per-candidate sizeless workspace refresh | - | - | 3.38 |
| ba83651d | board plan uses a zero-size measurer | 1.54 | 1.29 | 3.41 |
| 40f78c87 | TALA node/edge pointer slots as relaxed atomics | 1.54 | 1.30 | 3.28 |

Geomean at 40f78c87: Dagre 0.82x, ELK 0.78x (faster than Go), TALA 1.43x.
The TALA total is dominated by `tpmjs_architecture` (26 s vs Go's 6.3 s) and
`arrowhead_scaling` (11 s vs 3.3 s). Its profile at 40f78c87: atomics still
~15% self (`OwnerGroupRecord::fields` cloning the group's node header under a
lock via `active_cluster_index_slow` → `topology_vessel_reference`, then
routing `VisibilityGraph::point` / `SharedPortNode::point`), and
`translate_node_with_children` at 7.1% self, which rebuilds BTreeSets, scans
every cluster and sequence in a fixpoint, and scans `nodes` linearly per
external child on every move.

### The median ratio hides a bimodal split (2026-10-26)

The per-engine "wall ratio" above is a median of per-case ratios. On the bench
VM, Go pays about 25 ms of runtime startup per run and uses ~1.25 cores (GC and
runtime threads), while d2-rs uses exactly 1.00. As a result d2-rs is 2-4x
*faster* on small boards (e.g. `measured/empty-shape`: 6 ms vs 25 ms), but
about 2x *slower* on large ones (Dagre `tpmjs_architecture`: 486 vs 230 ms,
`grid_large_checkered`: 377 vs 44 ms, i.e. 8.6x). The same data from e39197c2
on all 12 cases, summarised three ways:

| Engine | Median | Geomean | Summed time | Large-case geomean (Go > 60 ms, n=5) |
|---|---:|---:|---:|---:|
| Dagre | 1.10 | 0.97 | 1.93 | 1.88 |
| ELK | 1.09 | 0.94 | 1.80 | 1.73 |
| TALA | 1.45 | 1.42 | 3.88 | 3.03 |

`perf-bench.py` now prints the geomean and summed ratios as well. The target is
the summed and large-case figures, not the median.

### Clean-host: L87/L88 validation folds + I66 incident walk (2026-10-26)

Bench VM (t2a-standard-4), 12 cases/engine, 5 interleaved repeats, Rust/Go
wall / CPU / RSS. Before is 99779fb7, after is with these changes:

| Engine | Before | After |
|---|---|---|
| Dagre | 1.20 / 1.06 / 0.50 | **1.10 / 0.95 / 0.49** |
| ELK | 1.22 / 0.99 / 0.51 | **1.09 / 0.89 / 0.51** |
| TALA | 1.53 / 1.63 / 0.56 | **1.45 / 1.56 / 0.58** |

All 36 rows are byte-identical, and the e2e scoreboard still has 933 SVG + 11
error matches. Building with `-C target-feature=+lse` (inline instead of outline
aarch64 atomics) measured 1.21 / 1.22 / 1.52 against 1.20 / 1.22 / 1.53 without
it, so the outline helpers are not the cost. The atomics in the profile are
`Shared` / `RwLock` traffic itself.

### Clean-host benchmark: GCP t2a-standard-4, linux/arm64 (2026-10-26)

The MacBook was too loaded and noisy for timing, so perf benching moved to a
spot `t2a-standard-4` (Ampere Altra, 4 dedicated cores, no SMT, Debian 12).
Go oracle built from the pin for linux/arm64; d2-rs release builds of 9ee6f9c4
("head") and of 9ee6f9c4 plus the L86/I65/run-106 lane work ("wip").
`perf-bench.py` now interleaves Go and Rust per repeat after one untimed
warm-up (`--warmup`), so host drift lands on both sides; 12 cases/engine,
5 repeats, medians. Rust/Go wall / CPU / peak RSS:

| Engine | head | wip |
|---|---|---|
| Dagre | 1.67 / 1.35 / 0.52 | **1.22 / 1.08 / 0.50** |
| ELK | 1.72 / 1.46 / 0.55 | **1.22 / 1.00 / 0.53** |
| TALA | 2.00 / 2.06 / 0.59 | **1.55 / 1.64 / 0.59** |

All 36 rows byte-identical in every repetition. On this clean Linux host
Rust is still slower than Go on wall time for every engine (the earlier
sub-1.0x macOS diagnostic figures do not reproduce here); memory stays about
half of Go's. e2e scoreboard with the wip build on the VM: 933 SVG + 11 error
matches, unchanged.

A full `cargo test --workspace --release --no-fail-fast` on the VM found
suites that had been failing unnoticed, because a plain `cargo test` stops at
the first failing binary:

* `d2-rs-language` `ir_compiler`, `public_graph_structure` and
  `public_measured_state` held v0.9.0 expectations that master supersedes.
  Re-pinned in 14a7e3d0, with master diagnostics verified by the CLI.
* The vendored `rustybuzz` shape example did not build where `c_char` is
  unsigned (aarch64 Linux). Fixed in 14a7e3d0.
* **Open, Linux only:** `d2-rs-export` test
  `positioned_unicode_callouts_and_appendix_match_complete_source_pixels`
  passes on macOS but its `top-left-text2-DEFAULT-linkfalse` RGB hash differs
  on linux/arm64. The raster path calls system libm (`sin_cos`, `atan2`, `tan`,
  `powf`, `log10`, `hypot` in `d2-rs-export`), and those results differ
  between platform libms. PNG parity is therefore only qualified on macOS.
  The fix is portable implementations of those functions plus a Linux run of
  the PNG scoreboard.
* `d2-rs-isometric` `artwork_api_tests::replay_original_calls` needs
  `.reference/codex-runs` and fails on any host without it. This is an
  environment dependency, not a parity failure.

### Run 106: reuse SVG board plans and defer unused hashes (2026-10-26)

Continued the complete brief through **Update 39**. For the supplied
`real_world/spyre_encoder` profile, ordinary SVG rendering now reuses the CLI's
already validated board traversal instead of compiling/lowering it again for
subtree hashing. Own-board hash bytes are produced only when descendants need
them. Direct rendering retains its planning fallback; the explicit
`diagnostic-traces` target-byte request keeps its eager, once-only behavior.
Mapping: master `d2cli/main.go:514–557`, `d2target/d2target.go:168–217,410–423`.
No arithmetic, FMA or output traversal order changes. No commits.

**Isolated paired profile:** 20 alternating warmed Spyre render pairs in one
executable preserve complete SVG bytes. The render body falls **42.42 →
31.78 ms**, **25.1% less time**; repeated board planning costs **10.30 ms**.
This excludes the initial CLI plan, which remains. Unused own-hash generation
costs **0.20 ms**. Remaining inclusive stage medians: source layout (compile,
measure and layout) **12.42 ms**, config extraction **2.12 ms per call** (six
hydration calls), hash metadata **2.07 ms**. This stage profile supplies fresh
measurements; no new stack percentages are claimed. Reusing a scoped config/
compiled-source result is the next measured Lane F candidate.

Fixed 12-case/engine **diagnostic** Rust/Go medians, wall / CPU / peak RSS:

| Engine | Before | After |
|---|---|---|
| Dagre | 1.32 / 1.10 / 0.59 | **0.90 / 0.92 / 0.58** |
| ELK | 1.22 / 1.06 / 0.64 | **0.72 / 0.88 / 0.63** |
| TALA | 1.59 / 1.88 / 0.72 | **1.57 / 1.79 / 0.71** |

All **36 rows and every repetition** produce nonempty, stable Go-identical
SVG bytes. Whole-CLI changes include L86/I65 and host load; only the paired
render profile isolates plan reuse. No new release-profile or universal
speed-superiority claim; F105's release recommendation stands.

**Zero regressions:** all **966 e2e rows identical**, **933 SVG + 11 expected
errors**; raster **317/313/313** with every classification unchanged; CLI
**67/70** under explicit shared-clock/release controls, **51 goldens**;
cli-diff **119/119**; isometric SVG **870 + 138 exact errors**, both PNG paths
**20/20**. CLI retains one skip and two unsupported injected writers. **253
CLI tests pass, 15 ignored**; the manual profile also passes separately.
The full suite includes two new selection/trace contracts. Other full tiers
are inherited; inventory remains **653/673 named**, no new upstream coverage.
The final own Cargo build equals the frozen CLI used by all guards.

Evidence: `.reference/codex-runs/run106/` (`qualified.json`, before/after e2e
and perf reports, deltas, `source-profile.json`, test logs, `main.patch`, PNG
capture and retirement/compression ledgers). Handoff: `report-run106.md`.
Deleted **967.141 MiB** allocated path footprint of 1,740 exact-golden SVG
copies and empty private scratch folders; expected goldens, request streams,
inputs and complete errors remain. Verified Applesauce **0.5.30 LZFSE** saves
**120.281 MiB** on 23 retained files, preserving bytes, executability/modes,
owners, sizes, mtime and other xattrs; clone backups removed. PNG's **11 inputs
and 413 captured bindings**, original SVG replacement hashes and retained
executables verify after compression. Gross savings differ from APFS physical
reclamation and concurrent checkout changes. No bulk moved.

Final allocated/apparent GiB: checkout **23.38/41.00**, `.reference` **12.76/27.54**, `target` **9.14/11.73**; **47.30 GiB free**; all ceilings pass
(`disk-final.json`). Whole scans include hidden paths, `.git` and every
apparent hard link; non-atomic amid other lanes. No builds or captures
followed this audit.

### Lane L86: graph-session property lookup and borrowed scalar text (2026-10-26)

Continued the full brief through **Update 39**; the earlier API queue is
complete. Targeted the supplied `real_world/spyre_encoder` front-end profile.
Graph lowering now indexes field names within one immutable session, retaining
separate first-field/first-map candidates and reserved-keyword quoting. A
nested miss never retries a later map. Lowercase ASCII query keys are borrowed;
other keys use the pinned Go Unicode fold table. The index is discarded on
return/error, restored across nesting, and never shared with mutable canonical
property forks or reused across public lowering calls. Class/SQL helpers also
avoid constructing temporary AST query nodes. Scalar text reads under source
read guards and copies directly into the result, preserving box precedence,
first-interpolation-slot behavior and one-invalid-byte UTF-8 presentation.
Source: master `d2ir/d2ir.go:1106–1150`, `d2ast/d2ast.go:585–608,1492–1517`.

**25 alternating complete compiler calls per frozen binary**, every output
byte compared, improve median wall **23.95 → 21.69 ms** and CPU
**21.58 → 19.45 ms** (ratios **0.906 / 0.901**). The initial index trial did
not establish an improvement and remains separately recorded; the final version
avoids keyword checks on misses and intermediate scalar buffers. A fresh
50-iteration stage profile gives parse **0.944 ms**, IR **1.003 ms**, graph
lowering/projection **9.767 ms**. Graph construction remains about **83%** of
these stages; no fresh symbol-level stack percentages are claimed. This API
probe excludes actual font measurement, layout and rendering.

Diagnostic CLI Rust/Go ratios, 12 cases/engine and three repetitions:

| Engine | Wall before → after | CPU before → after | RSS before → after |
|---|---:|---:|---:|
| Dagre | 0.89× → **0.87×** | 1.17× → **0.92×** | 0.59× → **0.59×** |
| ELK | 1.08× → **1.07×** | 1.15× → **0.82×** | 0.64× → **0.63×** |
| TALA | 1.83× → **1.63×** | 2.04× → **1.78×** | 0.72× → **0.71×** |

All **36 rows / every repetition** match complete Go SVG bytes. These shared
CLI snapshots include concurrent Lane F/I changes and varying host load;
the engine ratios do not isolate this Lane L change. This run measures the
diagnostic profile; F105 separately recommends release for headline benchmarks.

**Zero regressions:** all **966 e2e classifications** remain unchanged
(**933 SVG + 11 expected errors**); language retains **915 live goldens / 445
editor text+error results** and master compiler retains **70/70** original
calls. E2e/language deltas **fixed 0 / regressed 0**; compiler gains/regressions
empty. Original **930 golden hashes / 70 compiler records** verify. Fresh
inventory remains **653/673 named, 20 raw without**. No new golden/API coverage
is claimed; other full tiers are inherited. Direct validation passes **72
current core-source tests + 2 exact index-module tests**. The latter uses a
minimal owner containing the same arena/index fields; integration is covered
by the complete graph/compiler scoreboards. The queued Cargo library run was
withdrawn through terminal control after more than 25 minutes; no fresh full
language-library unit pass is claimed. Preliminary test-setup failures remain
in the logs. The idle failed warm language executable was retired explicitly
and must be rebuilt successfully through Cargo.

Evidence: `.reference/codex-runs/lang86/` (`source.json`, `source.patch`,
`frontend-qualified.json`, `frontend-stages-qualified.json`, `perf-before.json`,
`perf-qualified.json`, e2e/language delta logs and `compiler-delta.json`).

Retired **280 redundant compiler replay files** and the idle failed language
warm runner: **25.148 MiB allocated**. The warm path's prior
qualified hash and its unqualified replacement are explicitly retired;
**86 fixture hashes**, all 930 original golden hashes and the original
70-call input/output closure verify. Rebuild that warm path through Cargo
before reuse. Verified Applesauce **0.5.30 LZFSE** on 20 retained files saves
**122.773 MiB**; bytes, modes, ownership, sizes, mtime and other
xattrs verify, with all clone backups removed. Compressed current core/index
tests pass again; compressed compiler bytes and a complete Go SVG also match.
Gross allocated path reduction **147.922 MiB**, distinct
from APFS physical reclamation and concurrent checkout changes. Exact paths,
hashes and replacements: `cleanup.json`, `warm-runner-retirement.json`, both
compression plans/ledgers and `storage-summary.json`. No bulk moved.

Final allocated/apparent: checkout **23.41/40.99 GiB**, `.reference` **12.75/27.53 GiB**, `target` **9.17/11.73 GiB**; **48.45 GiB free**. All ceilings pass
(`disk-final.json`, independent non-atomic scans including hidden paths and
apparent hard-link duplicates). No own builds or parity captures followed
this final audit. Read-only recheck: `uv run --no-project python
.reference/codex-runs/lang86/check.py`. Handoff: `report-lang86.md`.

### Lane L85: allocation-free front-end field comparisons (2026-10-26)

Continued the complete brief through **Update 39**, which supersedes the
finished API-test queue. Targeted the supervisor's `real_world/spyre_encoder`
profile: `property_scalar_inherited` / IR field lookup and scalar allocation.
Comparisons now stream bytes through Go's pinned Unicode fold table, with an
ASCII fast path and one-byte malformed-UTF-8 decoding. Field keys avoid copied
buffers; shared string storage is borrowed within read guards; interpolated
scalar materialization copies once. Graph scalar/presence/range queries accept
borrowed unquoted paths instead of constructing temporary AST nodes.

Source mappings: master `d2ir/d2ir.go:1106–1150` and
`d2ast/d2ast.go:589–608,1520–1555`. Public field lookup keeps live source order,
reserved-keyword ToLower/quoting and duplicate traversal behavior. Existing
compiler index winner/mutation rules and all arithmetic remain unchanged.
No public lookup cache was introduced; a graph-session index remains future
work if the next profile supports it.

On **25 alternating complete compiler calls per binary** for Spyre, every
output byte agrees: median wall **42.83 → 22.72 ms**, CPU
**40.70 → 20.79 ms** (about **1.9× faster**). This pre-measurement API probe
excludes font measurement, layout and rendering. A 50-iteration stage profile
finds parse **0.92 ms**, IR compilation **1.00 ms**, graph lowering/projection
**11.80 ms**; the latter remains the largest front-end stage. macOS `sample`
could not attach; no fresh symbol-level profile is claimed.

Fresh `perf-bench` medians against Go, 12 cases/engine, three repetitions:

| Engine | Wall before → after | CPU before → after | RSS before → after |
|---|---:|---:|---:|
| Dagre | 1.96× → **0.85×** | 1.82× → **1.09×** | 0.55× → 0.58× |
| ELK | 2.32× → **1.00×** | 2.15× → **1.14×** | 0.61× → 0.63× |
| TALA | 2.75× → **2.00×** | 2.54× → **2.11×** | 0.70× → 0.71× |

These whole-CLI numbers include concurrent Lane F/I changes and changing
system load; they are not an isolated Lane L speedup. All **36/36 rows** and
every repetition have identical SVG bytes. The harness now uses isolated child
resource usage and monotonic wall time because `/usr/bin/time -l` hits a
sandboxed sysctl; stderr EOF avoids timeout-polling quantization. Each repetition
checks exit/output hashes and removes stale output first. The supervisor
included the shared harness correction in **db2dea0d** during this run; Lane L
made no commits.

**Zero regressions:** session-start and final e2e retain all **966** classifications
(**933 SVG + 11 expected errors**), language retains **915 live goldens / 445
editor text+error results**, master compiler retains **70/70** original calls;
all deltas **fixed 0 / regressed 0**. **174 Rust tests** pass (71 core, 67
language library, 36 focused integration), plus two harness tests, Ruff and
scoped formatting. Tests cover every Unicode scalar's pinned fold key,
malformed bytes, live field reordering/name mutation, reserved quoting and
nested duplicate lookup. Compressed native tests, complete compiler bytes and
CLI SVG also verify. Inventory remains **653/673 named, 20 raw without**;
no new upstream coverage or golden-board gain is claimed. Other full tiers
are inherited.

The concurrent TALA test build briefly pushed `target/` above 10 GiB. Retired
**37 obsolete idle, unbound language/core test executables** (**274.21 MiB
allocated**), preserving 14 bound warm paths, all 930 original golden hashes,
70 original compiler records and 86 fixture files. Three subsequently rebuilt
warm-runner hashes have explicit historical retirement overlays. Removed
**142 identical replay/smoke copies** (**0.914 MiB**). Verified Applesauce
**0.5.30 LZFSE** on 19 retained files saves **116.73 MiB**; bytes, modes,
owners, sizes, mtime and non-compression xattrs verify, and clone backups are
removed. Gross allocated path savings **391.86 MiB**, separate from APFS
physical reclamation and concurrent checkout changes. No active file or hard
link rewritten; no bulk moved.

Evidence: `.reference/codex-runs/lang85/` (`perf-before-qualified.json`,
`perf-qualified.json`, `frontend-perf.json`, `frontend-stages.json`, the three
scoreboard deltas, `retirement.json`, `warm-runner-retirement.json`,
`compression.json`, `cleanup.json`). Fresh whole-checkout gate and final disk
audits: `disk-gate.json`, `disk-final.json`. Handoff: `report-lang85.md`.

Final allocated/apparent GiB: checkout **23.30/40.97**, `.reference`
**12.06/26.63**, `target` **9.61/12.43**; **55.70 GiB free**. All ceilings
pass. Whole-checkout and independent subroot scans include hidden paths and
every apparent hard link; measurements are non-atomic. Later Lane I65 edits
are outside the frozen, hash-identified binaries qualified here. No own
artifact-heavy work followed the final audit.

### Run 105: shared fonts and lazy glyph bounds, performance phase (2026-10-26)

Read the complete brief through **Update 39**, which replaces the exhausted
API queue with performance work. The supplied `real_world/spyre_encoder`
profile attributes about 14% to repeated font parsing/outlining. Native
`TrueTypeFont` now shares immutable metrics through Arc, caches each bundled
face once per process, and calculates each glyph's outline once on demand.
The CLI and both SVG/scene Markdown rulers share that registry. Custom fonts
own their input bytes and retain the original validation/fallback behavior.
Mapping: master `lib/textmeasure/textmeasure.go:134–153` and
`fontface.go:30–58`. No arithmetic, FMA or output iteration order changes.

**Zero output regressions:** diagnostic and thin-LTO release each preserve
all **966 e2e rows**, **933 SVG + 11 expected errors**. Raster remains
**317/313/313**, CLI **67/70** under the explicit shared-clock/release profile
(one skip, two unsupported writers), **51 goldens**, cli-diff **119/119**,
isometric SVG **870 + 138 exact errors**, and both PNG paths **20/20**.
**139 text tests pass**, including every bundled glyph's eager/lazy bounds,
owned-input lifetime and concurrent measurement. No new golden/API coverage
is claimed. Frozen binaries identify the shared snapshots used by each tier;
later other-lane edits are outside these observations.

Fixed 12-case/engine benchmark, Rust/Go ratios (**wall / CPU / peak RSS**):

| Engine | Before | Diagnostic | Release |
|---|---|---|---|
| Dagre | 2.31 / 1.77 / 0.55 | 1.01 / 1.18 / 0.58 | 1.14 / 1.09 / 0.54 |
| ELK | 2.58 / 2.17 / 0.61 | 1.26 / 1.18 / 0.63 | 1.05 / 1.08 / 0.59 |
| TALA | 2.63 / 2.37 / 0.70 | 1.90 / 2.00 / 0.71 | 1.90 / 2.11 / 0.67 |

All **36 rows**, every repetition, produce complete nonempty Go-identical
bytes in each profile. These are integrated front-end/font/TALA snapshots,
not isolated F105 speed-ups. The isolated 20-font-request witness preserves
dimension bits and falls from **18.07 ms to 0.247 ms** on its first lazy batch.
Use **release** for headline benchmarks: paired Rust CPU medians are 3–6%
lower and RSS about 7% lower than diagnostic. Separate Go runs/host load make
normalized ratios noisy; these results do not establish universal superiority
to Go. TALA remains slower. The final benchmark uses the shared harness's
precise child wall/CPU/RSS measurements and checks all three repeats.

Fresh stack profiling is unavailable: installed Instruments templates fail
on missing `vmtrack`, and CPU Counters aborts before recording. No new stack
percentages are claimed. The supplied profile's repeated board-plan/subtree
hash compilation remains the next F candidate, subject to fresh profiling.
Inventory stays **673 / 653 named / 20 unsupported or infrastructure**.

Evidence: `.reference/codex-runs/run105/` (`perf-*.json`, `final-deltas.json`,
CLI/raster deltas, source patch/hashes, tests and explicit retirement ledgers).
The first release e2e guard exhausted its capture budget and is **not a pass**;
after retiring 1,740 successful SVG copies against exact pinned golden hashes,
the fresh `release-qualified` guard passes all 966 rows. Temporary concurrent
build growth crossed the target ceiling; own builds stopped, verified idle
test compression and a fresh complete `disk-recovered.json` audit preceded
resumption. Final retention/disk details are recorded in `report-run105.md`.
No commits by F105; other lanes and `29` preserved.

Final allocated/apparent GiB: checkout **22.85/40.29**, `.reference` **12.22/26.80**,
`target` **9.16/11.74**; **54.17 GiB free**; all ceilings pass. Removed
**2,404.555 MiB** allocated path footprint; credited verified LZFSE
savings **125.453 MiB**, excluding the shared warm-runner saving
superseded by its subsequent rebuild and the metadata revision. Exact path/hash
ledgers and the original observation remain; no bulk moved. Gross path savings
are separate from physical APFS extents and concurrent net checkout change.

### Lane I64: TALA ancestry allocation, performance phase (2026-10-26)

Read the full brief through **Update 39**, which supersedes the completed
layout-test queue. Its supplied CPU profile puts about 79% of TALA time in
the sized optimizer/scoring/spatial path. In `owner_score_frame.rs`, four
ancestry/deduplication walks now retain their first sixteen identities on
the stack, falling back to the existing BTreeSet for deeper chains. This
removes shallow-walk heap allocation without changing traversal, live owner
reads, nil precedence, first-cycle errors, cancellation polls or arithmetic.
Mapping: pinned master `layoutgraph/layout.go:314–331`,
`node.go:2354–2410`, `placementcost/edge_length.go:875–926`.

**Zero output regressions:** all **966 e2e rows exactly unchanged**,
**933 SVG + 11 expected errors**, including **308 TALA SVGs**;
**313/313 TALA raster cases** match through the complete PNG inventory/pixel
gate. **15 frame tests and two cancellation-interval tests pass**, including
a new 40-level ancestry/cycle test exercising the fallback. The final own
rebuild also repeats all **966 unchanged SVG rows**. Raster/performance bind
the earlier frozen shared CLI; later concurrent lane changes are separately
identified in `qualified.json`. No new golden-board gain is claimed.

All **36 perf-bench rows** remain byte-identical to Go. Fresh measured TALA
Rust/Go medians: wall **2.69× → 2.01×**, CPU **2.54× → 2.16×**, RSS
**0.69× → 0.72×**. These whole-CLI measurements include concurrent front-end
and font changes and varying host load: **no isolated I64 speed-up or claim
that every case improved**. The sandbox denies `time -l`'s sysctl query;
the retained wrapper uses child `wait4` metrics with the same perf-bench
corpus/summary and streaming limits. A temporary-file rename race in its
first after-run guard was corrected and the run repeated successfully.
Fresh CPU sampling remains unavailable (`sample` inspection denied;
Instruments recording fails); the supplied profile is inherited, not a
post-change measurement. TALA remains slower than Go; further optimizer
allocation work and a fresh host profile remain open.

Inventory remains **673 added / 653 named / 20 raw without**; assigned layout
API queue **31/31 named**, no new observation credit. No commits or edits to
other lanes' source. Evidence and explicit supervisor file list:
`.reference/codex-runs/report-iso64.md`, `iso64/qualified.json`, deltas,
input/source hashes and warm-runner retirement overlay.

Observed temporary test codegen exceeded the target ceiling (**10.047 GiB**).
New parity launches paused; active files were preserved. Compiler completion
retired its temporary objects, and a fresh whole-checkout audit proved every
ceiling passed before tests resumed (`storage-gate.json`, `disk-pretests.json`).
Deleted **27.531 MiB** of own disposable artifacts; verified Applesauce
**0.5.30 LZFSE** saves **33.055 MiB** more. Bytes, modes, owners, mtime,
sizes and other xattrs verify; clone backups removed. Warm runner was already
compressed and was not rewritten. Exact paths/rollback: `cleanup.json`,
`compression-plan.json`, `compression.json`; no bulk moved. An unidentified
old linker executable was preserved rather than retired without a verified
replacement. Initial failed setup/guard/compression attempts remain explicit.

Final allocated/apparent GiB: checkout **24.32/41.87**, `.reference`
**13.16/27.64**, `target` **9.67/12.49**; **54.17 GiB free**. All ceilings
pass (`disk-final.json`). Shared-checkout scans are non-atomic; gross path
savings do not imply equivalent APFS extent reclamation. No builds or parity
captures followed the final audit.

### Run 103: Lane F queue exhausted under the current API scope (2026-10-26)

Re-read the complete brief through **Update 38**, newest reports/messages and
current status. Fresh inventory remains **673 added / 648 named / 25 without
named observations**. Lane F has **zero actionable functions**: its three
remaining CLI functions require cyclic/shared Go pointer targets that the
owned native `TargetDiagram` cannot represent; the fourth tests excluded
legacy external-plugin discovery. They remain unsupported/out of scope,
never native passes. `run103/scope.json` records each original function,
reason and source hash. Per the brief's stop rule, this lane stops without
changing runtime code, broadening the API, or repeating their captures.

The required session-start e2e guard, using the existing qualified binary,
still gives **933 SVG + 11 expected errors**: all **966 classifications
identical** to run102, **fixed 0 / regressed 0**. No builds or new algorithm
tests; other tiers are inherited. Evidence:
`.reference/codex-runs/run103/{inventory,scope,session-e2e,e2e-delta}.json`;
handoff `report-run103.md`. No commits; existing work and `29` preserved.

Final allocated/apparent GiB: checkout **22.78/40.32**, `.reference` **11.78/26.13**, `target` **9.51/12.45**; **58.75 GiB free**. All ceilings pass (`run103/disk-final.json`). No obsolete corpus/build artifacts created or deleted. Verified Applesauce 0.5.30 LZFSE on the two new completed JSON reports saves **0.422 MiB allocated**; bytes, modes, owners, sizes and other xattrs verify, temporary clone backups removed. Exact paths/rollback and deletion ledger: `run103/{compression-plan,compression,cleanup}.json`. No bulk moved.

### Lane L83: original features, icon ownership, PDF and snapshots (2026-10-26)

Continued the full brief through **Update 38**, superseding Update 26. Batched
**eight original Go functions**: **63/63 native API calls match** on initial
and final replay, **regressed 0**, plus **three Go infrastructure functions**
recorded N/A. The native calls comprise **52 feature checks, one full scene /
icon ownership result, eight complete two-page PDFs and two original isometric
SVG renders**. No renderer algorithm fix or new golden-board gain is claimed.

`d2-rs-cli/src/layout_features.rs` shares the CLI's former Dagre/ELK checks
with the named engine entry point from master
`internal/d2layoutfeatures/features.go:22–92`; `main.rs` delegates before
layout. The original two functions cover all six engine spellings and four
unknown names, comparing complete errors. The icon observer preserves authored
URL credentials explicitly across Go's private-field JSON boundary and compares
the complete scene and both URL/Userinfo ownership checks. This names the fix
already landed in a2f4c63a; it is not another runtime fix.

The original `internal/testutil` PDF fill test compares all **16 pages** as
complete document bytes under the existing explicit **UTC epoch 946782245 /
FPDF catalog-sort** profile. Its original image, page-stream, background order,
header contrast and physical breadcrumb-destination assertions all remain active.
`TestIsometricSVGSnapshot` names its two actual Render calls; original snapshot
acceptance, inventory and skip-flag assertions separately pass in Go. Actual
source shows `TestIsometricBoards` is a Go board-name/stale-snapshot helper,
not a renderer invocation. It and PNG `TestSnapshotPixels` / `TestPixelRoundingBound`
are N/A infrastructure, never native passes or permission to relax exact raster
comparison. This corrects Update 38's assumption about the first helper.

**Guards:** e2e remains **933 SVG + 11 expected errors**, all **966 rows
identical**, before/master deltas **fixed 0 / regressed 0**. **251 CLI tests
pass / 14 existing ignored**, **15 old/new harness tests** pass. The shared
resolver remains **89/89 events** before and after compression. Compressed
native replay reproduces all **63** calls and the compressed CLI executes.
Original test files restore byte-for-byte; complete retained input/output and
source/runner hashes verify. Other full scoreboards are inherited. The first
score attempt preceded the executable; its failed log remains, and the valid
source capture was replayed with `--reuse` after the build completed.

Fresh inventory **673 / 648→653 named / 25→20 raw without**; own **+5**.
Lane L is **15/23 named, eight raw without**: **four unsupported identity/type
functions and four Go infrastructure functions**, zero actionable unnamed
functions. Existing partial SVG/LaTeX observations remain partial. Evidence:
`.reference/codex-runs/lang83/` (`scope.json`, `qualified-api.json`, deltas,
`source.json`, `check.py`, original capture); master registry
`renderer-residual-api.json`. Shared-runner retirement explicitly preserves
F102's historical hash and input/output closure with its current 89-event
replacement. No commits; own main edit is only feature-check extraction/wiring.
Other-lane PNG files and `29` preserved.

Deleted **114.758 MiB allocated path footprint**, including only this
run's redundant executables and identical replay copies; exact paths and
replacement/rebuild records: `cleanup.json`. Applesauce **0.5.30 LZFSE** with
verification saves **116.297 MiB**; bytes, modes, owners, apparent
sizes, mtime and other xattrs verify. No active files or hard links rewritten;
per-file APFS clone backups removed immediately. Rollback and exact paths:
`compression-plan.json`, `compression.json`;
`applesauce decompress --verify -- <exact path>`. Gross path savings
**231.055 MiB**, separate from physical APFS reclamation;
whole-checkout allocated net **+19.551 MiB** includes shared builds/caches.
No bulk moved.

Final allocated/apparent sizes: checkout **22.80/40.42 GiB**, `.reference` **11.83/26.23**, `target` **9.48/12.45**; **57.60 GiB free**; all ceilings pass
(`disk-final.json`). Independent scans include hidden paths, `.git` and every
apparent hard link and are non-atomic. No builds or parity captures followed
the final audit.

### Run 102: original layout option snapshots and concurrent resolvers (2026-10-26)

Read the full brief through **Update 38**, newest reports/messages and current
status. Batched all five queued original Dagre/ELK option, TALA option/seed
snapshot and concurrent resolver functions: **5/5 functions, 89/89 events**
match. These comprise **13 option snapshots, 36 resolver lookups and 40
complete layout graph/error results**, including eight concurrent native
workers for each engine. The production CLI now shares the owned, immutable
typed option/seed snapshot in `layout_resolver.rs`; resolved handles retain
their options independently of the resolver. Source: master
`d2cli/layout.go:51–70` and its Dagre/ELK/TALA option closures. No layout
algorithm fix or new golden-board gain is claimed.

The combined observer retains the original Go test bodies/assertions.
Input-only native replay compares full graph JSON, complete errors and every
numeric bit, including signed zero. Its explicitly limited flat measured
graph projection preserves all original attributes/references. Initial
**72/89** omitted ELK default label positions and TALA's `Object.Text` font
size in that new projection; correcting those mappings gives **89/89**,
**regressed 0**. These were adapter omissions, not native algorithm failures.
The negative report and capture remain. Trace test sources are restored.
Capture limits: streaming **1 GiB total / 128 MiB file / 20 GiB free**,
600-second Go and 120-second native deadlines, fresh writable replay paths.

**Zero regressions:** before and rebuilt after e2e remain **933 SVG + 11
expected errors**; all **966 classifications identical**, before/master
deltas **fixed 0 / regressed 0**. **251 CLI tests** and **9 capture-gate
tests** pass. Compressed native replay, source/runner hashes and complete
retained input/output inventories verify. Other full scoreboards are
inherited. Inventory snapshot **673 / 638→648 named / 35→25 raw without**:
own **+5**, concurrent Lane L **+5**; CLI **9→4**. Remaining CLI names are
three raw cyclic/identity-target contracts and the excluded legacy-plugin
test. They remain unqualified; no N/A native passes were inferred.

Evidence: `.reference/codex-runs/run102/` (`resolver-qualified.json`,
`resolver-initial.json`, both original captures, deltas, `scope.json`,
`source.json`, `check.py`); registry `cli-layout-resolver-original.json`;
handoff `report-run102.md`. No commits. Own `main.rs` edit is minimal;
concurrent renderer/PNG work and `29` preserved. Deleted **6** superseded
own paths, **28.352 MiB allocated**, including the F100 executable;
`cleanup.json` explicitly retires historical run100/run101 binary bindings
and records replacements/rebuild instructions. Original inputs, expected
outputs and the negative baseline remain. Verified Applesauce 0.5.30 LZFSE
saves **70.477 MiB**, preserving bytes, modes, owners, apparent sizes and
non-compression xattrs; no active files/hard links rewritten, clone backups
removed. **98.828 MiB gross allocated path savings**, separate from physical
APFS reclamation and concurrent checkout net change. Exact paths/rollback:
`compression-plan.json`, `compression.json`.

Final allocated/apparent GiB: checkout **22.78/40.32**, `.reference`
**11.78/26.13**, `target` **9.51/12.45**; **59.13 GiB free**. Every ceiling
passes (`run102/disk-final.json`); checkout allocated net **−91.547 MiB**
includes concurrent work/shared caches. Independent scans include hidden
paths, `.git` and every apparent hard link and are non-atomic. No builds or
parity captures followed the audit.

### Run 101: original isometric private-network policy calls (2026-10-26)

Read the complete brief through **Update 38**, newest reports/messages and
current status. Completed the remaining original function in
`d2cli/isometric_integration_test.go`,
`TestIsometricAssetsHonorPrivateNetworkPolicy`: **6/6 paired calls match**
across denied/allowed SVG, PNG and PDF exports. Both sides make zero requests
under the default policy and one request with explicit permission. Complete
errors, source-file inventories/modes and emitted bytes compare; SVG/PDF are
byte-exact under the existing explicit v0.9.0/shared UTC clock profile, PNG
passes the unchanged complete decoded-pixel gate. No runtime fix or new
golden-board gain is claimed.

`scripts/private-asset-scoreboard.py` and its Go observer retain the original
test body/assertions against the real native CLI and independently check Go.
Paired calls share the same loopback URL, input path and asset bytes. Native
errors pass through the actual `xmain.TestState` API boundary, after verifying
and reversing the command-level terminal prefix; full diagnostics remain in
the capture. The initial **3/6** result compared raw terminal errors with
Go's API wrappers; this was a transport defect, retained as negative evidence,
not a native failure/fix. Existing PDF instrumentation is validated and
restored exactly; original test source bytes are restored. Streaming limits:
**1 GiB total / 128 MiB file/stdout/stderr / 20 GiB free**, finite deadlines.

**Zero regressions:** frozen F100 runtime reused without a new build; before
and after e2e remain **933 SVG + 11 expected errors**, all **966 classifications
identical**, with **fixed 0 / regressed 0** against both before and master.
**21** new capture failure-gate tests and **38** existing raster-gate tests
pass. No Rust production code changed; other full tiers are inherited.
Original inputs, output inventories, hashes and comparisons verify after
compression. Inventory snapshot: **673 added / 637→638 named / 36→35 raw
without**, CLI **10→9**. This completes the remaining function in one test
file, as permitted by Update 37. Next actionable batch: the five original
Dagre/ELK/TALA option, seed-snapshot and concurrent-resolver functions;
pointer-cycle cases remain explicitly unqualified.

Evidence: `.reference/codex-runs/run101/` (`private-qualified.json`, current
`private-capture-sealed/`, `private-negative.json`, e2e reports/deltas,
`scope.json`, `check.py`); registry `cli-private-asset-original.json` in the
master scoreboard directory; handoff `report-run101.md`. No commits or
shared Rust edits. Removed **58** own obsolete paths, **0.750 MiB allocated**;
replacement evidence is recorded in `cleanup.json`. Verified Applesauce
0.5.30 LZFSE saves **0.980 MiB** on idle retained evidence, preserving bytes,
modes, owners, apparent sizes and non-compression xattrs. No hard links or
active files rewritten; clone backups removed. **1.730 MiB gross allocated
path savings**, distinct from physical APFS reclamation and concurrent
checkout net change. Exact paths/rollback are in the compression ledgers.
Final allocated/apparent GiB: checkout **22.83/40.23**, `.reference`
**11.84/26.12**, `target` **9.51/12.37**; **58.36 GiB free**, every ceiling
passes (`run101/disk-final.json`). Allocated checkout net **+150.504 MiB**
includes concurrent work/shared caches. Scans include hidden paths, `.git`
and every apparent hard link and are non-atomic. No builds or new parity
captures followed that audit.

### Lane L82: original SVG, target, sketch and LaTeX admission (2026-10-26)

Continued brief **Update 38** with eight original functions: **41 calls** plus
one Go source-syntax ratchet. Native matches **29→35**, **zero regressions**:
four textual sketch calls no longer panic, and two missing typed-attribute
calls now return Go's complete errors. `d2svg/sketch.rs` follows master
`d2renderers/d2sketch/sketch.go:340–367`: parse legacy attributes, reject
renderer-owned/duplicate names and zero-value attributes before drawing,
and pass typed attributes directly to the SVG element. No new golden gain.

The original board-limit function (**6/6**, including both HashIDs) and
non-finite route function (**1/1**) already match. Raw-isometric admission
is **15/18**, and LaTeX escaping/comments **7/8**: five cyclic/reused Go
pointer calls and one invalid-UTF-8 string remain **unsupported** across the
batch. The original serialization ratchet passes in Go but is **N/A** for
native API parity. Thus three functions fully match, two remain partial,
two are unsupported, and one is infrastructure; none is silently excluded.
The flat capture preserves pointer identities and exact non-finite bits;
original Go assertions stay active and complete native results compare.

**Final guards:** all **966 e2e classifications identical**, **933 SVG + 11
expected errors**, before/master deltas **fixed 0 / regressed 0**. Raw SVG
remains **939/952**, with the same **11 stale Go matches + 2 Go decode errors**.
All **265 renderer tests**, **16 old/new harness tests**, and the previous
**15 renderer calls** pass. Final compressed runners reproduce all **35**
representable calls. The typed handoff was corrected during source review;
intermediate evidence and explicit executable retirement records remain.
Other full tiers are inherited; this does not establish full D2 parity.

Inventory snapshot: **673 added / 637→643 named / 36→30 raw without**;
**+5 own / +1 concurrent F101**. Lane L's 23 assigned names now have **10 named /
13 without**; named coverage does not qualify the two partial functions.
Next: remaining image clone/raster admission, icon identity, named e2e/PNG,
layout-feature and PDF observations. Evidence: `.reference/codex-runs/lang82/`,
master registry `renderer-admission-api.json`, and `report-lang82.md`.
No commits or own CLI-main edits; concurrent layout work and `29` preserved.

Deleted **130.547 MiB allocated path footprint**. Verified Applesauce
0.5.30 LZFSE saves **145.695 MiB** across **51 current retained paths**,
excluding retired/overwritten intermediate binaries from that total.
Bytes, modes, owners, apparent sizes and non-compression xattrs verify;
no active files or hard links rewritten and clone backups removed immediately.
Exact paths, replacements and rollback are in `cleanup.json`,
`native-retirement.json` and the compression ledgers. No bulk moved.

Final allocated/apparent sizes: checkout **22.78/40.32 GiB**, `.reference` **11.78/26.13**, `target` **9.52/12.45**; **59.13 GiB free**; all ceilings pass
(`disk-final.json`). Whole-checkout allocated net **+132.676 MiB**, including
shared builds/concurrent work; path savings are separate from physical APFS
extent reclamation. Scans include hidden paths, `.git` and every apparent
hard link and are non-atomic. No builds or parity captures followed this audit.

### Lane L81: original animation, viewport and LaTeX calls (2026-10-26)

Read the full brief through **Update 38**, latest reports/messages and STATUS.
Five original functions / **15 complete calls** now match, **13→15**,
**fixed 2 / regressed 0**. Both failures were in `d2animate.Wrap`:
`d2svg/animate.rs` now validates before traversal and defaults absent padding
to 100, following master `d2animate.go:46–57`. The precise malformed-child
error and full default-padding SVG match. `TargetDiagram::wrap_svg_frames`
exposes that shared implementation. The two original viewport-rounding
functions (six calls) and LaTeX `TestInputLimits` (seven calls) already match;
no scene/math algorithm edits. Original assertions stay active. Complete
SVG bytes, matrices, dimensions, failure return values and errors are retained;
no normalization or fixture-selected outputs.

**Zero regressions:** e2e stays **933 SVG + 11 expected errors**, all **966
classifications identical**; baseline and before/after deltas both zero.
**264 renderer tests**, the native **15-call fixture**, and **7 capture-gate
tests** pass. Compressed executable reruns pass too. An initial native fixture
assertion distinguished JSON `0` from `0.0`; corrected to compare typed
`Matrix` values, preserving every f64 value; negative log retained. Other full
scoreboards are inherited, with no new golden-board or full-parity claim.

Inventory snapshot: **673 added / 628→633 named / 45→40 raw without**.
Update 38's “21” Lane L heading enumerates **23 names**; all 23 are retained
in `lang81/scope.json`, **5 newly named / 18 still without observations**.
Its PDF name is actually in `internal/testutil`, not `lib/pdf`; no credit or
N/A classification was inferred. Next largest renderer group is four SVG
functions; target bounds/admission remain separate. Evidence:
`.reference/codex-runs/lang81/`, `renderer-contract-api.json` in the master
scoreboard directory, and `report-lang81.md`. No commits or shared CLI-main
edits; other lanes and `29` preserved.

Deleted **61.371 MiB allocated path footprint**: three superseded small
captures, transient inventories, a duplicate baseline CLI snapshot verified
against retained F99 bytes, and duplicate successful replay output.
`cleanup.json`, `binary-retirement.json` and `native-retirement.json` record
replacements, historical hashes and rebuild instructions. Verified Applesauce
0.5.30 LZFSE saves **99.332 MiB** across **25** idle evidence/executable paths;
bytes, modes, owners, apparent sizes and non-compression xattrs verify, no
hard links rewritten, clone backups removed immediately. Exact paths and
rollback (`applesauce decompress --verify -- <exact path>`) are recorded in
the two compression ledgers. Gross path savings **160.703 MiB**; whole-checkout
allocated net change **+221.379 MiB**, including shared builds/concurrent work;
path savings are not a claim about physical APFS extent reclamation.

Final allocated/apparent GiB: checkout **22.72/40.10**, `.reference`
**11.69/25.96**, `target` **9.55/12.40**; **59.13 GiB free**, all ceilings
pass (`lang81/disk-final.json`). Scans include hidden paths, `.git` and every
apparent hard link and are non-atomic. No builds or parity captures followed
the final audit.

### Lane L79: Update 37 a–h CLI queue self-stop (2026-10-26)

Read the full brief through **Update 37**, newest reports/messages and STATUS.
The latest assignment supersedes Update 26 and the earlier n–z assignment:
`d2cli/{board_output,export,fmt,help}_test.go`. Fresh inventory finds **21/21
assigned master-added functions already named; zero uncovered or actionable**.
Repository snapshot: **673 added / 628 named / 45 raw without**. Under Updates
34–37, self-stop without runtime edits, builds, captures, repeated regression
guards or fresh parity claims. Existing observation limits remain; no new
native passes or golden-board gain. Evidence and deletion ledger:
`.reference/codex-runs/lang79/scope.json`; handoff: `report-lang79.md`.
Removed only the transient full inventory, **0.301 MiB allocated**; no commits,
other-lane changes or cleanup outside this run. Fresh disk audit: `lang79/disk.json`.

Disk allocated/apparent GiB: checkout **22.49/39.86**, `.reference` **11.61/25.86**, `target` **9.39/12.28**; **60.27 GiB free**; all ceilings pass.

### Run 100: original watch tests and built-in layout flag order (2026-10-26)

Read the full brief, including the initial Update and new **Updates 37–38**,
latest reports/messages and STATUS. Batched all four original functions in
`d2cli/isometric_watch_test.go` plus `TestBuiltinLayoutFlagOrder`
(`layout_test.go:91–125`). **Four of five functions qualify; 16/17 observations
match exactly**: four source-mode reload SVGs, six layout invocations, five
direct-preview SVGs and the empty-folder error. The cyclic-pointer half of
`TestIsometricWatchPreviewRejectsInvalidTrees` stays **unsupported**; the
complete function is non-passing. Owned native targets cannot represent that
Go pointer graph. No new golden gain or runtime bug fix is claimed.

Master `isometric_watch.go:19–95` maps to the production static preview
copy/relink/painter extracted as `render_target_preview`, with shared
navigation-path encoding. The input-only raw-target replay expands acyclic
identity graphs into owned values; original SVG bytes, errors and unchanged
target/options compare. Static previews exclude file-animation master IDs
and dark themes. Original Go assertions stay active; the reload and layout
flag-order assertions also execute against the real native CLI. Both builds
use an explicit `v0.9.0` version stamp, without SVG normalization. The
subprocess boundary records the native CLI's intentional **SIGINT exit 1**
(`main.rs`, `watch::was_interrupted`) before mapping a requested, completed,
error-free shutdown to API cancellation. Initial shutdown-adapter negatives
remain recorded; they are not a runtime failure or fix.

**Zero regressions:** e2e **933 SVG + 11 expected errors**, all **966
classifications identical** before/after; before/master deltas **fixed 0 /
regressed 0**. Existing complete watch corpus **20/20**, unchanged against
master. **251 CLI tests** pass (13 ignored replay entries); **35 harness
failure-gate tests** pass. Complete SVG/output inventories and modes compare;
all 20 watch stages revalidate SVG/PNG/manifests through the existing gates.
Source/runner/capture hashes and original trace-source restoration verify,
including after compression. Other full tiers are inherited. Missing/extra
outputs, timeouts and exhausted budgets remain failures. Captures use
**1 GiB total / 128 MiB file / 20 GiB free** and finite deadlines.

Inventory snapshot **673 added / 637 named / 36 raw without**: this run adds
**four** names, concurrent Lane L work adds five. CLI residuals **14→10**.
Next: original isometric private-network policy calls, then remaining folder/
cycle admission and mutable/concurrent layout-resolver contracts. Evidence:
**run100/{watch-after,watch-delta,watch-regression,scope,source}.json**,
**source.patch**, before/after e2e and **check.py**; registry
**cli-watch-original.json**; handoff **report-run100.md**. Prior missing raw
replay dispatch remains non-passing coverage evidence, not a runtime 0→6 fix.
No commits or `main.rs` edits; concurrent renderer/PNG work and `29` preserved.

Deleted **15** own obsolete files, **2.922 MiB allocated path footprint**.
Exact duplicate capture bindings have explicit hash/reconstruction retirement
entries in **cleanup.json**; latest inputs/expected/native outputs remain
complete. Verified Applesauce **0.5.30 LZFSE** on **186** idle retained paths
saves **77.301 MiB** (**80.223 MiB gross path savings**, distinct from physical
APFS extent reclamation and concurrent checkout net change). Bytes, modes,
owners, apparent sizes and non-compression xattrs verify; no hard links or
active files rewritten, clone backups removed immediately. Compressed CLI
startup, native replay and full retained-output checks pass. Exact paths and
rollback: **run100/{compression-plan,compression}.json**;
`applesauce decompress --verify -- <exact path>`.

Final allocated/apparent GiB: checkout **22.65/40.10**, `.reference` **11.68/25.96**, `target` **9.48/12.40**; **58.75 GiB free**.
Every ceiling passes (**run100/disk-final.json**). Whole-checkout allocated
net change **+161.410 MiB** includes concurrent Lane L work and shared caches;
measurements are non-atomic. No builds or new parity captures followed the audit.

### Run 99: original append-path containment calls (2026-10-26)

Read the complete brief through **Update 36**, newest reports/messages and
STATUS; continued Lane F a–m/SVG. Original
`TestAppendBoardOutputNameStaysContained` now has **9/9 exact native
observations** (master `d2cli/board_output_test.go:115–155`,
`board_output.go:79–119`). Seven unsafe names, the direct parent-traversal
rejection and hidden `.svg` output path match complete Go paths/errors.
Original assertions remain active. The CLI planner and replay share
`contained_board_output_component` / `ensure_path_within`; existing lexical
collision-key cleaning is extracted unchanged. The SVG-only input adapter
uses the existing output-root behavior. No runtime bug fix or new golden
board gain is claimed; other extensions, arbitrary byte paths, symlink
resolution and Windows filepath behavior remain unqualified.

**Zero regressions:** frozen F98 before / rebuilt F99 after e2e both
**933 SVG + 11 expected errors**, all **966 classifications unchanged**;
standard before/master deltas **fixed 0 / regressed 0**. The original path
corpus is now **75/75**; its previous **66/66** remain exact. Both original
board-link calls still match; all **72 target-link cases** remain
**60 source + 12 error matches**, unchanged against run98. All **251 CLI
tests** pass (12 ignored replay entry points), plus **34 board-output** and
**89 CLI-mode harness tests**. Changed-function formatting/diff checks pass.
Other full tiers are inherited. The first native run caught an empty-root
translation mistake (`filepath.Abs("")` uses cwd; Rust `absolute("")`
rejects it); corrected before final qualification, with the negative log
retained. An initial harness invocation without Pillow was rerun with its
required dependency and passes. The old runner's missing append dispatcher
is retained as negative evidence, not a runtime 0→9 fix. Trace source bytes
are restored; capture guards remain **1 GiB total / 128 MiB file / 20 GiB
free**, finite deadlines and fresh writable outputs.

Fresh inventory: **673 added / 628 named / 45 raw without**; Lane F
**88 assigned, 70→71 named, 18→17 without**; whole CLI residuals **15→14**.
Next: original watch preview/navigation, static GIF preview and source-mode
reload calls, prioritizing any observed failure. Prior cyclic/raw-target
and mutable-layout limits remain uncredited. Evidence: **run99/**,
**scope.json**, **source.json**, **source.patch**, **path-delta.json**,
e2e/target-link deltas and **check.py**; registry
**cli-board-path-api.json**; handoff **report-run99.md**. The shared warm
runner's replacement hashes and prior rebuild revision are explicit in
**native-runner-retirement.json**. No commits; unrelated PNG work and `29`
preserved.

Deleted four own disposable files and the superseded F98 snapshot,
**56.297 MiB allocated path footprint**;
verified Applesauce **0.5.30 LZFSE** saves **68.977 MiB** on **152** idle
retained evidence/executable paths (**125.273 MiB gross path savings**, distinct
from physical APFS clone extent reclamation or checkout net change). Bytes,
modes, owners, apparent sizes and non-compression xattrs verify; no hard
links rewritten; clone backups removed immediately. Compressed execution,
75-call replay and full input/output verification pass. Exact paths,
replacement records and rollback: **run99/{cleanup,compression-plan,compression}.json**.
The F98 snapshot retirement verified all prior 72 input/output inventories
and the original source at commit `7b744fc5`; hashes/rebuild instructions
remain in **run99/binary-retirement.json**. No bulk moved. Final allocated/apparent GiB: checkout **22.49/39.86**, `.reference` **11.61/25.85**, `target` **9.39/12.28**; **59.51 GiB free**.
Every ceiling passes (**run99/disk-final.json**); audited checkout allocated
net change is about **+10.4 MiB**. Whole scans include hidden paths, `.git`
and every apparent hard link; measurements are non-atomic. No builds or
parity captures followed the final audit.

### Run 98: original node/connection board-link calls (2026-10-26)

Read the full brief through **Update 36**, newest reports/messages and STATUS;
continued Lane F's a–m/SVG queue. Original master
`TestRelinkIncludesConnections` now has **2/2 exact native observations**
(`d2cli/board_output_test.go:742–771`, `main.go:817–875`). Complete target
JSON, errors and escaped URL strings match, including both shape and
connection links and the direct parent-relative path call. Original Go
assertions remain active. Extracted the existing production node-style and
graph edge-link loops as `relink_document_links`, with shared
`relative_board_output_link` / `board_output_link` escaping. The replay
transports links into those actual storage/loops, preserving other target
fields. No runtime bug fix, normalization or new golden-board gain.
Recursive boards, error paths and arbitrary raw filepath inputs remain
outside these two calls. Old runners lacking the replay correctly stay
non-passing; this is observation coverage, not a runtime **0→2** fix.

**Zero regressions:** frozen F97 before / rebuilt F98 after e2e both
**933 SVG + 11 expected errors**, all **966 classifications unchanged**;
standard deltas against before and master **fixed 0 / regressed 0**.
The whole **72-case target-link corpus** remains **60 source + 12 error
matches**, with zero classification changes. All **251 native CLI tests**
pass (12 ignored replay entry points), **89 CLI-mode harness tests** and
**32 board-output harness tests** (seven new link gates) pass. Original
board-path replay remains **66/66**. Modified helpers/adapter formatting and
`git diff --check` pass. Other full tiers are inherited. Trace test bytes
are restored exactly. Fresh writable outputs, capture guards **1 GiB total /
128 MiB file / 20 GiB free** and finite deadlines remain; missing native
entry points/responses cannot pass.

Fresh inventory: **673 added / 627 named / 46 raw without**; Lane F
**88 assigned, 69→70 named, 19→18 without**; whole CLI residuals **16→15**.
Next: original append-path containment, then watch residuals. Previous
cyclic/raw-target and mutable-layout limitations stay uncredited.
Evidence: **run98/{link-after,link-delta,target-link-delta,scope,source}.json**,
e2e/test logs and **check.py**. Registry: **cli-board-links-api.json**;
handoff: **report-run98.md**. Shared warm-runner replacement has explicit
old/new hashes in **native-runner-retirement.json**. No commits; unrelated
files, including `29` and the PNG-scoreboard work, preserved.

Deleted **0.977 MiB** of three own transient files; verified Applesauce
**0.5.30 LZFSE** saves **68.773 MiB** on **146** idle retained evidence and
executable paths (**69.750 MiB gross path savings**, separate from checkout
net change). Bytes, modes, owners, apparent sizes and non-compression xattrs
verify; no hard links rewritten; APFS clone backups removed immediately.
Compressed execution/replay and full retained-input/output checks pass.
Exact paths, rollback and deletion ledger:
**run98/{compression-plan,compression,cleanup}.json**. No bulk moved.

Final allocated/apparent GiB: checkout **22.48/39.85**, `.reference` **11.60/25.84**, `target` **9.40/12.28**; **59.89 GiB free**. Every ceiling passes (**run98/disk-final.json**). Net checkout allocated change **+44.633 MiB**, distinct from gross path savings. Independent scans include hidden paths, `.git` and every apparent hard link; shared-volume scans are non-atomic. No builds or parity captures followed the audit.

### Run 97: raw-byte board names, finishing unfinished run96 (2026-10-26)

Re-read the complete brief through **Update 36**, newest reports/messages,
STATUS and the accepted workflow. Adopted the pending run96 change after
static comparison with pinned master `d2cli/board_output.go:29–35,50–54`.
`board_output_component` now accepts original bytes, preserving portable
valid UTF-8 and hashing every other name with SHA-256. The test adapter
calls that production helper directly. Original `TestBoardOutputComponent`
improves **60/62 → 62/62**; with the four unchanged collision calls, the
path scoreboard improves **64/66 → 66/66**, **two fixed / zero regressed**.
Only the two original `0xff` determinism results change. No lossy decoding,
fixture changes, test exclusions or new golden-board gain. This qualifies
the raw-name helper, not arbitrary invalid-UTF-8 document/CLI transport.

**Zero regressions:** frozen F95 before / rebuilt F97 after e2e both
**933 SVG + 11 expected errors**, all **966 classifications unchanged**.
Standard deltas against before and master: **fixed 0 / regressed 0**.
All **251 native CLI tests** pass (11 ignored replay entry points), plus
**95 harness tests**, **12 original SVG export calls** and **four original
transactional/unsafe-name publication calls**. Modified-function/adapter
formatting and `git diff --check` pass; pre-existing unrelated full-file
formatting differences were preserved. Other full tiers are inherited.
Reused hash-verified original Go captures with all original assertions
active; each native replay uses a fresh writable path. An existing replay
name was correctly refused before retrying with a unique run97 name.
Capture guards remain **1 GiB total / 128 MiB file / 20 GiB free**, with
finite deadlines. Original trace sources remain unchanged.

Inventory: **673 added / 626 named / 47 raw without**; Lane F **88 assigned,
68 → 69 named, 20 → 19 without**, CLI residuals **17 → 16**. Next: original
`TestAppendBoardOutputNameStaysContained` and `TestRelinkIncludesConnections`,
then watch residuals. Cyclic/raw-target and mutable-layout limitations remain
uncredited. Evidence: **run97/{path-before,path-after,path-delta,scope,source}.json**,
e2e deltas, guard/test logs and **check.py**; registry **cli-board-path-api.json**;
handoff **report-run97.md**. The old/new warm-runner hash replacement is
explicit in **native-runner-retirement.json**. No commits; other lanes and
`29` preserved.

Deleted **0.605 MiB** of own transient files; verified Applesauce **0.5.30
LZFSE** saves **82.582 MiB** on 20 exact idle retained paths (**83.188 MiB
gross path savings**). Bytes, modes, owners, sizes and non-compression xattrs
verify; no hard links rewritten, clone backups removed. Compressed replay
and retained input/output checks pass. Exact paths/rollback and deletion
ledger: **run97/{compression-plan,compression,cleanup}.json**. Final disk
audit: **run97/disk-final.json**. No bulk moved.

Final allocated/apparent GiB: checkout **22.43/39.77**, `.reference` **11.58/25.82**, `target` **9.37/12.22**; **61.42 GiB free**. Every ceiling passes (**run97/disk-final.json**). Net checkout allocated change **+42.512 MiB**, separate from gross path savings. Independent complete scans include hidden paths, `.git` and every apparent hard link; shared-volume measurements are non-atomic. No builds or parity captures followed the final audit.

### Run 95: original SVG board-link provenance calls (2026-10-26)

Continued Lane F's a–m/SVG queue after reading the full brief, including the
initial Update through **Update 36**, and the newest handoffs. Two original
master functions now match: **5/5 link-rebase calls, four followed by Render**
(`d2cli/isometric_svg_test.go:131–148,176–207`). Original Go assertions remain
active. Complete rebased target JSON, unchanged source target, errors and
raw rendered SVG bytes match, including escaped node/connection links and
`./root{,.layers,.scenarios,.steps}.svg` URL provenance. No normalization,
runtime behavior fix or new golden gain.

`isometric_image.go:247–299` maps to the existing production loop extracted
as `isometric_cli::relink_svg_board`. The input-only test adapter translates
Go's local path-map IDs to Rust's full-source IDs before calling that loop.
Comparison reuses the language scoreboard's type-aware JSON equality.
Root-link mutation, allocated-empty-slice identity and failure paths are not
exercised by these original inputs and remain unqualified.

**Zero regressions:** frozen F91 before / newly built F95 after e2e both
**933 SVG + 11 expected errors**, all **966 classifications unchanged**;
standard delta against before and master **fixed 0 / regressed 0**. All
**12 existing original SVG export calls**, **251 native CLI tests** (11
explicitly ignored replay tests) and **89 CLI-mode harness tests** pass.
Changed-file formatting/diff checks, retained input/expected/native hashes
and exact trace-source restoration verify. Other full tiers are inherited.
Capture guards: **1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.

Fresh inventory: **673 added / 625 named / 48 raw without**. Lane F:
**88 assigned; 66 → 68 named; 22 → 20 without**; CLI residuals **19 → 17**.
Next: original connection-relink/board-output path tests, then watch residuals;
prior invalid UTF-8/cyclic-tree and mutable-layout limitations remain uncredited.
Evidence: **run95/**, **scope.json**, **source.json**, **check.py**;
registry **cli-svg-links-api.json**; handoff **report-run95.md**. Shared warm
runner replacement is explicit in **native-runner-retirement.json**. No
commits; other lanes and `29` preserved.

Deleted only this run's transient inventories/authoring scripts and a redundant
post-compression response; exact ledger **run95/cleanup.json**. Verified
Applesauce **0.5.30 LZFSE** on 17 idle retained evidence/executable paths saves
**76.609 MiB**. Bytes, modes, owners, sizes and non-compression xattrs verify;
no hard links rewritten, clone backups removed, compressed native replay and
CLI execution pass. Paths/rollback: **compression-plan.json**, **compression.json**,
`applesauce decompress --verify -- <exact path>`. Deleted **0.680 MiB**; gross path savings **77.289 MiB**.

Final allocated/apparent GiB: checkout **22.40/39.69**, `.reference` **11.58/25.80**, `target` **9.34/12.16**; **62.56 GiB free**. Every ceiling passes (**run95/disk-final.json**). Net checkout allocated change **+41.504 MiB**; independent complete scans include hidden paths, `.git` and every apparent hard link. Shared-volume scans are non-atomic. No builds or parity captures followed the final audit.

### Lane I62: assigned layout queue complete (2026-10-26)

Read the full brief through **Update 36** and newest handoffs. Fresh master-added
inventory: **31/31 assigned layout functions named; zero uncovered or actionable**.
Repository snapshot: **673 added / 623 named / 50 raw without**. Self-stop under
Updates 34–35; no runtime edits, builds, captures, repeated regression guards or
new parity claim. I61’s cancellation/projection coverage limits still apply.
Evidence and deletion ledger: **iso62/scope.json**; handoff: **report-iso62.md**.
Removed only the transient inventory, **0.301 MiB allocated**; other lanes and
`29` preserved. Final disk audit: **iso62/disk.json**.
Disk allocated/apparent GiB: checkout **22.36/39.61**, `.reference` **11.56/25.78**, `target` **9.31/12.10**; **62.18 GiB free**, all ceilings pass.

### Lane I61: original compound cancellation calls (2026-10-26)

Continued the full brief through **Update 36**; Update 35 assigns layouts.
The two remaining original `compound_interior_routes_test.go` functions now
have native observations: **5/5 calls**, comprising one successful refined
graph and four cancellations. Pinned master **6a7b48378**,
`engine/compound_candidate.go:17–72,112–145` and
`compound_routes.go:19–103` map to `snapshot::build_compound_candidate` and
`compound_routes::preserve_compound_routes`. Ordered geometry, label/icon
state, curve flags and original input preservation match exactly within the
I60 projection. All original Go assertions remain active. **No runtime fix
or new golden gain**; the observers and replay compile only under `cfg(test)`.

Cancellation compares the original `errors.Is` cause contract and null
candidate. Complete raw diagnostics remain visible. Go calibrates **966
Context.Err checks**, cancelling at **322/644/965**; native calibration uses
**11 existing stage/publication boundaries**, cancelling at **3/7/10**.
These independently apply the original fractions. Within-stage cancellation,
identical polls/counts and wrapped diagnostic text remain **unqualified**.
The successful native probe also compares the complete refined geometry
projection. Its infinite test incumbent bypasses optional score rejection,
matching the original pre-selection API. Caches, pointer identity and I60's
opaque metadata stay outside this observation.

**Eight harness gates, 13 existing compound tests, 13 existing interior calls,
ten existing optional-refinement calls and the explicit five-call replay pass**
on the rebuilt/compressed warm executable. Frozen I58 CLI e2e before/after:
**933 SVG + 11 expected errors**, all **966 classifications unchanged**,
standard delta **fixed 0 / regressed 0**. This guards unchanged runtime code;
concurrent CLI edits and other full tiers are not newly qualified. Native
bootstrap (zero matching tests), an initial Go-hook error-shadowing failure,
a rejected full-diagnostic schedule assumption and a refused existing replay
path remain visible negative/setup records; no original test is disabled.
The qualified refinement guard uses a fresh I61 output path.

Fresh inventory: **31 assigned; 29 → 31 named; 2 → 0 without**.
Repository snapshot: **673 added / 621 named / 52 raw without**, including
three concurrent CLI observations. Lane I has **zero actionable assigned
functions** and stops under the brief's self-stop rule. This is inventory
coverage with the stated limits, not full layout/API parity.

Evidence: **iso61/cancellation-qualified.json**, original calls/manifest,
source mapping/hashes, negative records, old-suite guards, e2e delta and
**check.py**. Registry: **tala-cancellation-api.json**; handoff:
**report-iso61.md**. Trace source restored; capture budgets **1 GiB total /
128 MiB file / 20 GiB free**, 600-second deadlines. The shared warm filename
has an explicit old/new runner retirement overlay. No commits; other lanes
and `29` preserved. Own temporary authoring scripts and transient inventories
were deleted; inputs, expected/native outputs and failure witnesses remain.
Verified Applesauce **0.5.30 LZFSE** preserves bytes, modes, owners and
non-compression xattrs; idle/link checks pass and clone backups were removed.
Exact ledger/savings/final audit: **iso61/cleanup.json**, **compression.json**,
**disk-final.json** and **report-iso61.md**. No bulk moved.

Final allocated/apparent GiB: checkout **22.35/39.60**, `.reference` **11.55/25.77**, `target` **9.31/12.10**; **62.56 GiB free**. Every ceiling passes (**iso61/disk-final.json**). Deleted **8 paths / 0.637 MiB**; verified compression of **7 retained files saves 313.023 MiB**. Gross path savings **313.660 MiB** differ from shared-checkout net allocated change **+64.348 MiB**. Independent complete scans include hidden paths, `.git` and every apparent hard link; shared-volume measurements are non-atomic. No builds or parity captures follow the final audit.

### Run 94: original local-icon and source-GIF CLI tests (2026-10-26)

Continued Lane F after reading the full brief through **Update 36** and the
newest reports. Two original functions now have native observations:
**four local SVG/PNG icon export calls and two source/flag GIF calls, 6/6**.
Pinned master `d2cli/isometric_surface_test.go:13–82` and
`isometric_test.go:126–148`; all original Go assertions remain active.
Native requests contain only original source/assets, arguments and environment
(empty PATH). SVG bytes match exactly; PNG uses the complete shared inventory
gate plus exact RGBA samples. GIF compares all composited frames, canvas,
delays and loop, with native source/flag encoded-byte equality checked too.
Go/Rust GIF encoder bytes differ and are not claimed identical.

The initial icon report is **3/4**, although all raw Go/native results are
identical. The CLI harness wrongly rejected the unchanged local `mark.png`
input as an unchecked emitted image. It now verifies original input bytes
and mode separately while retaining the complete emitted-image gate. The
same raw records become **4/4**; the initial negative remains in
**run94/surface-before.json**. No runtime change or golden-board gain.
Asset setup maps `isometric_image.go:78–101` to the existing native
`isometric_cli::resources`.

**Zero regressions:** fresh e2e before/after **933 SVG + 11 expected errors**,
all **966 classifications unchanged**, standard delta **fixed 0 / regressed 0**
also against the master baseline. Both use the unchanged frozen F91 executable;
this run changes no native code. **79 CLI-mode and 38 raster harness tests
pass**. Existing six image-export and 17 paged-export comparisons remain
unchanged under the corrected comparator. Other full tiers are inherited.
Capture limits: streaming **1 GiB total / 128 MiB file / 20 GiB free**, finite
600-second deadlines; original trace source restored and verified.

Inventory: **673 added / 623 named / 50 raw without**. Lane F **88 assigned;
64 → 66 named, 24 → 22 without**; whole CLI residuals **21 → 19**. Next:
original SVG board-link source-preservation/URL-provenance and connection
relink tests. Invalid UTF-8/cyclic inputs and mutable layout-option limits
remain explicitly uncredited. Evidence: **run94/** original calls/manifests,
reports, **deltas.json**, **scope.json**, **check.py**; registry
**cli-surface-icons-api.json**, **cli-source-gif-api.json**. No commits by
this run. Temporary audit cleanup and verified LZFSE pass recover **1.535 MiB**
across paths; exact deletion/compression ledgers are retained. Final disk
audit: **run94/disk-final.json**.

Final allocated/apparent GiB: checkout **22.36/39.61**, `.reference` **11.56/25.78**, `target` **9.31/12.10**; **62.18 GiB free**. Every ceiling passes (**run94/disk-final.json**). No builds or parity captures followed the audit.

### Run 93: original paged CLI tests (2026-10-26)

Continued Lane F's a–m/SVG queue after re-reading the full brief through
**Update 36** and the latest handoffs. Three original functions in pinned
master `d2cli/isometric_paged_test.go:43–162,202–238` now have named native
observations: **17/17 calls**, comprising 15 CLI invocations and two direct
link-rebasing calls. All original Go assertions remain active. Complete
PDF/PPTX bytes, stdout, file inventories and modes match under the established
explicit shared UTC clock/catalog-sort/release-version profile. Standalone
PNG boards pass the complete raster/manifest gate. No native runtime fix or
golden-board gain; application code only gains a `cfg(test)` declaration.

`isometric_paged.go:113–135` maps to existing native `isometric_paged::links`.
Its input-only test adapter observes target/URL, complete errors and unchanged
source links; box/tooltip are opaque preserved metadata. The initial capture
incorrectly put the clock only in `xmain.TestState.Env`, while Go's PDF hook
reads process environment. Six PDFs differed; **11/17 → 17/17** after fixing
that harness control. Original documents and the negative baseline remain
unchanged in **run93/paged-baseline.json**, with an explicit classification.

**Zero regressions:** fresh e2e before/after **933 SVG + 11 expected errors**,
all **966 classifications unchanged**, standard delta **fixed 0 / regressed 0**.
Both use the unchanged frozen F91 executable; concurrent layout edits are
outside this qualification. **251 CLI unit tests pass / ten ignored** and
**56 CLI-mode harness tests pass**. Formatting and diff checks pass. Other
full tiers are inherited. Capture limits: streaming **1 GiB total / 128 MiB
file / 20 GiB free**, 600-second deadlines; exact prior trace files restored.

Inventory snapshot: **673 added / 619 named / 54 raw without**. Lane F:
**88 assigned; 61 → 64 named, 27 → 24 without**; CLI residuals **24 → 21**.
Next: original SVG board-link/source-provenance and local surface-icon tests.
Evidence: **run93/** original calls/manifests, qualified/negative reports,
source mapping/hashes, deltas, tests, **scope.json**, **check.py** and
**report-run93.md**. Registry: **cli-paged-export-api.json**. No commits.
Own deletion ledger, verified LZFSE results and final whole-checkout audit:
**run93/cleanup.json**, **compression.json**, **disk-final.json**. Deleted **1.227 MiB**, verified LZFSE saves **37.031 MiB** on 40 idle retained files; gross savings **38.258 MiB**. Compressed-runner rebasing/path tests and retained input/output checks pass; clone backups removed.

Final allocated/apparent GiB: checkout **22.66/39.60**, `.reference` **11.55/25.77**, `target` **9.62/12.10**; **62.56 GiB free**. All ceilings pass (**run93/disk-final.json**). Independent complete scans include hidden paths, `.git` and every apparent hard link; concurrent-volume measurements are non-atomic. Checkout allocated net change **+376.441 MiB**, distinct from gross path savings. No builds or parity captures follow the final audit.

### Run 91: finish pending CLI errors and board-path observations (2026-10-25/26)

Re-read the full brief through **Update 36**, including the initial Update,
newest reports/messages and STATUS. Continued Lane F's CLI a–m/SVG scope;
inspected and completed unreported run90 work using its original captures.
No commits; other lanes' changes and `29` preserved.

Pinned master **6a7b48378**, `d2cli/isometric_image.go:234–236`: file-based
multiboard render/write failures include the selected subtree's local board
key. The pending `isometric_cli.rs` change qualifies against original
`TestIsometricMultiboardPublicationIsTransactional`: **2/3 → 3/3 calls**,
including complete error bytes. File preservation already matched; the fix
restores missing error context. Unsafe-name publication remains **1/1**, and
four original SVG export/failure functions remain **12/12**. Original Go
assertions and exact SVG/inventory/bytes/modes/stdout comparisons stay active.
Go captures/negative baseline remain under **run90/**; qualified replays are
under **run91/**. No new golden-board gain is claimed.

New `board-path-scoreboard.py` observes original tests at
`d2cli/board_output_test.go:17–124`. Both collision functions match all
**four complete errors**, including escaped duplicate names and file/directory
prefix collisions. Input-only replay calls production path planning and
validation. **60/62** name calls match; **two invalid UTF-8 calls remain
explicitly unsupported** by native `String` names. `TestBoardOutputComponent`
stays uncredited as a full match. Both valid replays agree: **64 matching /
two unsupported calls**, zero regressions. The initial replay selected no
tests on the old executable; its native-failed **before.json** remains as a
harness failure. Existing Rust code only gains a `cfg(test)` module; source
mapping and an original fixture are retained.

Fresh frozen-executable e2e before/after: **933 SVG + 11 expected errors**,
all **966 classifications unchanged**, standard delta **fixed 0 / regressed 0**.
The after executable includes the pending run90 CLI fix; later concurrent
layout edits are not inferred qualified. **251 native CLI tests pass, nine
ignored**; **47 CLI-mode + six path harness tests pass**. Other full tiers
are inherited. Streaming capture budgets: **1 GiB total / 128 MiB file /
20 GiB free**, finite deadlines. Original trace source is restored.

Fresh inventory: **673 added / 611 named / 62 raw without**. Six newly named
functions include the pending run90 captures; the other three new observations
are Lane I59's. Lane F: **88 assigned; 55 → 61 named; 33 → 27 without**.
Whole CLI residuals **30 → 24**. Next: three original paged CLI functions,
then remaining SVG link/source-provenance and surface-icon tests. Invalid
UTF-8/cyclic inputs and mutable Go layout-option limits stay visible.

Evidence: **run91/** reports, source/runner hashes, deltas, test logs and
**check.py**; three compact registry entries link to complete evidence.
Handoff: **report-run91.md**. Removed four own redundant replay requests /
formatting diff, **0.043 MiB allocated** (**cleanup.json**). No corpus
expected/native outputs, negative baselines, shared caches or other-lane
artifacts retired; no bulk moved. Applesauce **0.5.30 LZFSE saves 76.668 MiB**
on 11 exact idle retained files, including the CLI/test executables. Bytes,
modes, owners and non-compression xattrs verify; clone backups were immediately
removed. Both focused tests repeat after compression, and retained inputs /
expected outputs / native results verify. Exact targets/rollback:
**compression-plan.json**, **compression.json**;
`applesauce decompress --verify -- <exact path>`.
Gross savings **76.711 MiB** differ from checkout net change. Initial
concurrent target overage was cleared by Lane I59; **disk-initial.json** proves
all ceilings passed before captures/builds. Final storage: **disk-final.json**.

Final allocated/apparent GiB: checkout **22.66/39.94**, `.reference` **11.52/25.73**, `target` **9.66/12.48**; **62.56 GiB free**. Every ceiling passes (**run91/disk-final.json**). Six independent complete scans include hidden paths, `.git` and every apparent hard link; shared-volume measurements are non-atomic. Net checkout allocated change **-28.72 MiB**, distinct from gross path savings. No builds or parity captures follow the final audit.

### Lane I60: original compound-interior refinement calls (2026-10-26)

Continued the full brief through **Update 36**; Update 35 assigns layouts.
Five original `compound_interior_routes_test.go` functions now have native
refinement observations: **13/13 calls**. Pinned master **6a7b48378**,
`engine/compound_routes.go:19–103` maps to the existing native
`preserve_compound_routes`, `preserve_compound_interiors` and
`restore_compound_interiors`. Buses, nested routes, loops, unsafe interiors,
selected external curves, zero edge IDs and fixed label/icon state match.
All original Go assertions stay active; both input graphs compare after the
native call. **No runtime fix or golden gain**; only a `cfg(test)` module
declaration changes production files.

Scope: preceding compound placement/rerouting is **source-supplied input**.
Node label text and label-less percentages are opaque input metadata;
direction-map storage, caches, pointer identity and the two cancellation
functions are outside this observation. Captures remain unchanged. Native
requests spell integral geometry floats as `100.0` rather than `100`, with
exact JSON-value equality checked; no rounding or tolerance is introduced.
Initial adapter failures (missing replay, numeric spelling, omitted scalar)
remain explicit negative records, not production routing failures.

**Eight harness gates, 13 existing compound tests, ten existing optional
refinement calls and the explicit 13-call replay pass** on the rebuilt warm
test executable. Before/after e2e uses the same frozen I58 CLI:
**933 SVG + 11 expected errors**, all **966 classifications unchanged**,
standard delta **fixed 0 / regressed 0**. This guards unchanged production
code, not concurrent CLI edits. Other full tiers/unit tests are inherited.
Captures use streaming **1 GiB total / 128 MiB file / 20 GiB free** guards,
600-second deadlines and fresh writable replay paths. Trace source restored.

Fresh inventory: **31 assigned; 24 → 29 named; 7 → 2 without**.
Repository: **673 added / 616 named / 57 raw without**, including six
concurrent observations. Next: the original pre-cancelled and midflight
compound-candidate cancellation functions; exact names in **iso60/scope.json**.
Evidence: **iso60/interior-qualified.json**, original calls/manifest,
source mapping, negative records, guards, e2e delta and **check.py**;
registry **tala-interior-api.json**, handoff **report-iso60.md**. No commits.

Removed obsolete idle oracle-profile library/metadata caches and transient
inventories; retained executable hashes have an explicit warm-runner
retirement overlay. No corpus input/expected output or bulk moved/retired.
Verified Applesauce **0.5.30 LZFSE** preserves checksums, modes, owners and
non-compression xattrs; clone backups removed and compressed executable runs.
Exact deletion ledger, operation savings and final whole-checkout audit:
**iso60/cleanup.json**, **iso60/*compression.json**, **iso60/disk-final.json**
and **report-iso60.md**. Final allocated/apparent GiB: checkout
**22.28/39.57**, `.reference` **11.53/25.74**, `target` **9.28/12.10**; **57.60 GiB free**.
All ceilings pass. Deleted **388.922 MiB** and compressed **402.828 MiB**;
gross path savings **791.750 MiB**, checkout net change **-412.004 MiB**
including concurrent work. No builds/parity captures follow the final audit.

### Lane I59: original optional TALA refinement calls (2026-10-25)

Continued the full brief through **Update 36** (Update 35 assigns layouts),
after reading the newest I58/F89/L75/L77 handoffs. Fresh inventory:
**31 assigned; 21 → 24 named observations; 10 → 7 without**. Repository:
**673 added / 605 named / 68 raw without**. No commits or production changes.

The three original `d2talalayout/refinement_test.go` functions now have native
selection observations: **10/10 calls**. Pinned master **6a7b48378**,
`layout.go:308–345` maps to `snapshot::consider_optional_proposal`, the
`build_compound_candidate` score comparator and unconditional routing-refinement
publication in `race_seed_layouts`. A successful refinement with worse generic
score publishes; ordinary errors/panics fall back; direct/wrapped cancellation,
deadline and caller cancellation propagate exactly. Original Go assertions remain
active. Graph/sequence/binding payloads are **opaque**: this is acceptance and
publication evidence, not routing/serialization/pointer-identity parity. Errors
expose no value at the Rust `Result` boundary; Go's error-path selected-pointer
assertions are Go-only. The ordinary control tests the production comparator,
not execution of the compound-layout builder. **No bug fix or golden gain**.

**Seven harness gates, one existing optional-proposal test, 13 existing compound
tests and the explicit replay pass** on the rebuilt warm executable. The first
replay used an incorrect exact module filter, ran zero tests and was correctly
non-passing; the same captured calls pass after fixing the filter. Its negative
report is retained separately. Both e2e runs use the frozen I58 executable:
**933 SVG + 11 expected errors**, all **966 classifications unchanged**,
standard delta **fixed 0 / regressed 0**. This guards the unchanged production
path, not concurrent CLI edits. Other full tiers and the remaining native unit
tests are inherited. Trace source is restored. Captures use streaming
**1 GiB total / 128 MiB file / 20 GiB free** guards and 600-second deadlines.

Evidence: **iso59/** original calls/manifest, **refinement-qualified.json**,
scope, source mapping, test logs, e2e reports/delta and **check.py**; registry
**tala-refinement-api.json**. Historical warm test filename/hash retirement is
explicit in **native-runner-retirement.json**. Next: **seven functions in
`internal/engine/compound_interior_routes_test.go`**; no isometric sweep repeated.

Storage: a running test build temporarily exceeded the target ceiling;
parity captures paused until **disk-pre-capture-restored.json** proved every
ceiling passed. Exact deletions/hashes/rebuild records are in **cleanup.json**;
verified LZFSE records cover seven idle executables plus retained evidence.
Checksums, executable modes, owners and original non-compression xattrs verify;
all seven executables load after compression and clone backups are removed.
No corpus source/input/expected output retired and no bulk moved. Final disk
measurements and actual operation savings: **iso59/disk-final.json** and
**report-iso59.md**. Shared whole-tree scans are non-atomic and include hidden
paths/.git and every apparent hard link.

Final allocated/apparent GiB: checkout **22.69/39.84**, `.reference`
**11.51/25.70**, `target` **9.70/12.40**; **64.47 GiB free**. All ceilings
pass. Deleted **nine obsolete/cache/transient paths, 88.262 MiB allocated**;
verified Applesauce **0.5.30 LZFSE saves 457.176 MiB** on seven executables
and four retained evidence files. Gross path savings **545.438 MiB**;
whole-checkout allocated net change **−196.988 MiB**, including concurrent
work and new build outputs. No builds or parity captures follow the final audit.

### Lane I58: original grid measurement contracts (2026-10-25)

Read the full brief through **Update 35**, including the initial Update,
newest I56/L72/F87 reports/messages, STATUS/PLAN/workflow and arriving
F88/F89/L74 handoffs. Update 35 assigns Lane I the layout queue. Fresh
inventory: **31 assigned; 18 → 21 named observations; 13 → 10 without**.
Repository snapshot: **673 added / 602 named / 71 raw without**; its delta
also includes concurrent F/L observations. No commits.

The three original `d2grid/grid_integral_test.go` measurement functions now
have native numerical observations: **116/116 sessions, 110,596/110,596
measurements**. Complete interval bounds and IEEE-754 bits compare, including
NaN payloads, signed zero, infinities, fractional arithmetic and the exact
integer boundary. Pinned master **6a7b48378**, `grid_search.go:88–99` and
`layout.go:631–666,811–829`, maps to the existing native row-size/gap sums.
The runtime edit only extracts the existing gap loop into `grid_row_with_gap`
so replay uses production code. **No numerical bug fix or golden gain**.
Original Go prefix/cache allocation assertions remain active; native cache
layout/laziness is outside this explicitly numerical projection. A named
observation does not qualify every original assertion.

All **103 native layout tests pass, four ignored** (three existing plus the
explicit new replay). **Six new harness gates and six existing search gates
pass**. Existing search **443/443**, capacity **5/5**, and admission **18/18**
calls remain matched on the rebuilt warm test executable. Historical bindings
for its reused filename are explicit in **iso58/native-runner-retirement.json**.
Fresh rebuilt CLI e2e: **933 SVG + 11 expected errors**, all **966 rows
unchanged**, standard delta **fixed 0 / regressed 0**. Other full tiers
inherited. Captures use streaming **1 GiB total / 128 MiB file / 20 GiB free**
guards and finite deadlines; original trace source is restored.

Evidence: **iso58/** source/capture manifests, measurement and guard reports,
scope, test/build logs, e2e delta and **check.py**; handoff **report-iso58.md**.
Next: the **seven compound interior-route tests**, then **three optional
seed-refinement tests**. No completed isometric sweep was repeated.
Cleanup removed **four own reproducible inventories/source copies,
1.027 MiB allocated**; exact hashes/rebuild records are in **cleanup.json**.
Verified Applesauce **0.5.30 LZFSE saves 56.793 MiB**; bytes, modes, owners
and non-compression xattrs verify, clone backups were removed, and both
compressed executables still run. Gross path savings **57.820 MiB**, separate
from checkout net change. No corpus input/expected output, shared cache or
other-lane artifact retired; no bulk moved. Final audit: **iso58/disk-final.json**.

Final allocated/apparent GiB: checkout **22.87/40.28**, `.reference`
**11.49/25.69**, `target` **9.89/12.86**; **65.99 GiB free**. All ceilings
pass. Independent whole-tree scans include hidden paths/.git and every
apparent hard link; shared-volume scans are non-atomic. Checkout allocated
net change **+130.34 MiB**, distinct from operation/path savings above.
No builds or parity captures follow the final audit.

### Run 89: original CLI image exports (2026-10-25)

Read the full brief through **Update 35**, including the initial Update,
newest reports/messages and current STATUS. Continued Lane F's CLI/SVG scope.
Added `cli-mode-scoreboard.py --suite image-export`: original
`TestIsometricImageMultiboardExports`, `TestIsometricImageNativeCLI` and
`TestIsometricImageOptInKeepsNativePNG`, pinned master **6a7b48378**,
`d2cli/isometric_image_test.go:89–129,204–259`. All original assertions remain
active. All **six native calls match**, including **six PNGs and three GIFs /
108 frames**. This adds **three named original functions**, with **no runtime
fix or new golden gain**. The existing frozen F88 executable is hash-bound.

PNG uses the shared complete-output/manifest gate plus exact RGBA samples;
GIF compares every composited frame, delay, canvas and loop. File inventories,
modes, non-image bytes, stdout and complete terminal errors are checked.
Encoded image byte identity is not required. Native replay receives inputs
only. Two initial harness issues (inline Go temporary directories and a
suffixless stdout comparison path) are recorded separately; both valid native
replays contain identical results, **6/6 → 6/6, zero regressions**.

Fresh e2e guard on the retained F88 executable: **933 SVG + 11 expected
errors**, all **966 classifications unchanged**, standard delta **fixed 0 /
regressed 0**. This qualifies that executable, not concurrent layout edits.
Other tiers and Rust tests are inherited. **31 CLI-mode + 38 raster-harness
tests pass**. Capture limits: streaming **1 GiB total / 128 MiB file /
20 GiB free**, finite 600-second deadlines. Source is restored after capture.

Fresh inventory: **673 added / 602 named / 71 raw without** (includes the
other lanes' new observations). CLI **33 → 30**; assigned CLI/SVG residuals
**37 → 34**. Next by original output count: SVG static multiboard/stdout and
file-link tests, then unsafe-name integration. Unsupported cyclic inputs and
mutable Go layout-option contracts remain visible and uncredited.

Evidence: **.reference/codex-runs/run89/** original capture, both native
replays, **image-qualified.json**, source mapping, deltas, **check.py**, test
logs and cleanup/compression records. Registry adds **cli-image-export-api.json**.
No commits by this run; other lanes' files and `29` preserved. Removed **33 own
failed/redundant files, 0.238 MiB allocated**; expected/native bytes remain in
the complete capture/report and replay reconstructs inputs. Applesauce
**0.5.30**, verified LZFSE on **11 exact idle JSON files**, saves **0.344 MiB**;
hashes, modes, owners and original non-compression xattrs verify, and clone
backups were immediately removed. Gross savings **0.582 MiB**, separate from
concurrent checkout change. No shared-cache/corpus retirement or bulk relocation.
Final disk audit and handoff: **run89/disk-final.json**, **report-run89.md**.
Final allocated/apparent GiB: checkout **22.80/40.15**, `.reference`
**11.46/25.64**, `target` **9.86/12.78**; **64.85 GiB free**. Every ceiling
passes. Six independent full scans include hidden paths/.git and every
apparent hard link; measurements are non-atomic during concurrent lanes.
Net checkout allocated change is **+70.29 MiB**. No further builds or parity
captures follow the final audit.

### Lane L77: Update 36 CLI n–z self-stop (2026-10-25)

Re-read the complete brief through **Update 36** and newest reports. Fresh
inventory: **673 added / 602 named / 71 raw uncovered**. The current n–z
assignment contains **five functions: four named observations and one
unsupported cyclic-diagram input, zero actionable functions**.
`d2cli/raster_test.go:720` passes a self-referential Go diagram; native
`Diagram` owns its children. Both source/type hashes match L74's review.
The unsupported function remains visible and uncredited. Per Updates 34–36,
self-stop without runtime edits, builds or repeated parity guards; previous
regression evidence is inherited, with no fresh parity claim. Evidence and
deletion ledger: `.reference/codex-runs/lang77/scope.json`; handoff:
`.reference/codex-runs/report-lang77.md`. Removed only the transient full
inventory, **0.297 MiB allocated**. No commits or other-lane edits.

Disk allocated/apparent: . **22.89/40.28 GiB**; .reference **11.49/25.69 GiB**; target **9.89/12.86 GiB**; **65.99 GiB free**, all ceilings pass (`lang77/disk.json`).

### Lane L75: Update 35 self-stop (2026-10-25)

Re-read the complete brief through Update 35 and newest reports. Fresh
inventory remains **673 added / 602 named / 71 raw uncovered**; Lane L has
**zero actionable functions**, ten infrastructure N/A and one unsupported
cyclic-diagram input. All eleven review source hashes and the native target
type hash verify. No runtime changes, builds or repeated parity guards;
prior regression evidence is inherited. Per Updates 34–35, stop this lane.
Evidence: `.reference/codex-runs/lang75/scope.json`; handoff:
`.reference/codex-runs/report-lang75.md`. Only STATUS changes for review.

Disk allocated/apparent: . **22.81/40.16 GiB**, .reference **11.46/25.64 GiB**, target **9.88/12.80 GiB**; **65.23 GiB free**, all ceilings pass (`lang75/disk.json`). Deleted only the transient inventory, **0.297 MiB allocated**; ledger in `lang75/scope.json`.

### Lane L: Update 35 self-stop (2026-10-25)

Fresh required inventory: **673 added / 602 named / 71 raw uncovered**.
Lane L's assignment is unchanged from L74: **ten infrastructure N/A,
one unsupported cyclic CLI input, zero actionable functions**. All eleven
review source hashes still verify. Per brief Updates 34–35, no runtime edits,
builds or repeated parity guards; existing regression evidence is inherited.
Evidence: `.reference/codex-runs/lane-l-stop-update35.json`; requested handoff:
`report-lang1.md` (original L1 report preserved as `lang1-original-report.md`).

Disk allocated/apparent GiB: checkout **22.81/40.14**, `.reference` **11.46/25.64**, `target` **9.85/12.79**; **64.47 GiB free**, all ceilings pass. Deleted transient inventory: **0.297 MiB allocated**.

### Lane L74: original PNG calls and infrastructure triage (2026-10-25)

Read the complete brief through **Update 35**, including the initial Update,
newest L72/I56/F87 reports/messages and arriving F88 handoff. Update 35 moves
Lane L to `e2etests/{report,png}` and `d2js/js/ci`, then CLI test files n–z.
Fresh inventory: **673 added / 595 → 596 named / 78 → 77 without**.

`scripts/e2e-png-scoreboard.py` observes the original `TestPNG`
(`e2etests/png/render_test.go:19–109`) without replacing its assertions.
The snapshot directory is a private copy; pinned fixtures are never written.
All **20 original calls and 20 goldens** reproduce Go and match the rebuilt,
frozen native PNG runner with **exact decoded pixels**, complete nonempty
inventories and the prescribed raster gates. The initial replay qualified
the previous warm executable while its rebuild completed; its exact F62
executable binding remains. The intact Go capture was then replayed against
the frozen current runner. Both give **20/20, fixed 0 / regressed 0**.
This adds **one named original function**, not new golden matches or a
runtime fix. No Rust implementation files changed.

The other **ten assigned functions** test upstream infrastructure: six
HTML-review discovery/link tests, two snapshot-maintenance/comparison helpers
and two npm publishing credential-helper tests. They are explicitly
**not applicable to native API parity** under Update 30, with individual
source hashes/reasons in **lang74/review.json**. They remain visible and
uncredited in the raw inventory. The only uncovered n–z CLI function,
`TestRenderGIFRejectsCyclicBoards` (`raster_test.go:720`), receives a
self-referential Go diagram; native owned `Option<Vec<Option<Diagram>>>`
children cannot represent it. It remains **unsupported**, not passing.
No actionable unreviewed test remains in this assignment.

Required hash-bound F87 e2e guard: **933 SVG + 11 expected errors**;
standard delta **fixed 0 / regressed 0**, all **966 classifications**
unchanged. This guards that executable; other full tiers are inherited.
Build: diagnostic `isometric-png` example. **48 harness tests pass**:
38 `test_render_*.py`, six new original-call failure gates and four existing
PNG-tier gates. Captures have streaming **1 GiB total / 128 MiB file /
20 GiB free** guards and finite deadlines. Source/capture/runner hashes and
all retained input/expected-output bindings verify after cleanup/compression.

Evidence: **lang74/** `png-current.json`, `png-source/`, `png-delta.json`,
`review.json`, `scope.json`, e2e guard/delta, rebuild log and scripts.
Registry adds **e2e-png-original.json** and **e2e-harness-review.json**.
Cleanup removes **22 own redundant inventory/replay files, 2.508 MiB
allocated**; exact hash replacements are in **cleanup.json**. Applesauce
**0.5.30**, verified LZFSE on **16 exact retained files**, saves
**10.813 MiB**; hashes, modes, owners and non-compression xattrs verify,
clone backups removed immediately. No active/hard-linked files rewritten,
shared caches/corpus retired or bulk moved. Gross path savings **13.320 MiB**,
separate from checkout net change. Final independent whole-tree audit:
**lang74/disk-final.json**; handoff: **report-lang74.md**. No commits.

Final allocated/apparent GiB: checkout **22.79/40.16**, `.reference` **11.44/25.62**, `target` **9.87/12.81**; **65.23 GiB free**. All ceilings pass. Six independent full scans include hidden paths/.git and every apparent hard link; non-atomic during concurrent work. No builds or parity captures follow the audit.

### Run 88: source-configured isometric admission order (2026-10-25)

Read the complete brief through **Update 33**, including its initial Update,
newest F87/L72/I56 reports/messages and STATUS/PLAN/workflow. Continued Lane F.
The original private-network environment/help tests already match. The next
image-options/admission batch exposes two separate source-derived failures:
source `vars: {d2-config: {isometric: true}}` with `out.txt` and either
`--scale=0` or `--force-appendix` reports the wrong error.

| Gate | Before → after | Regressions |
|---|---:|---:|
| Original private-network environment/help calls | **2/2 unchanged** | **0** |
| Original image-options/failure calls | **19/19 unchanged** | **0** |
| Separate source-config admission witnesses | **0/2 → 2/2** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 unchanged** | **0** |
| Full CLI complete contracts / goldens, shared clock/release | **67 / 51 unchanged** | **0** |
| CLI diff, explicit shared clock | **119/119 unchanged** | **0** |
| Export / isometric mode calls | **16 / 29 unchanged** | **0** |

Source mapping: master **6a7b48378**, `d2cli/main.go:521–542` converts
the source-selected format before appendix and scale admission. Rust now
follows that order. Its earlier flag-selected checks remain source-mapped
to `main.go:279–305`. `isometric_image.go:20–65` also separates raw image
dimensions from SVG display sizing; the shared native sizing helper now
exposes raw results, including dimensions returned with errors. Nine direct
option calls use this new adapter in both measurements; the negative CLI
results use the retained F87 executable. No direct-option bug is claimed.

**249 CLI tests pass, eight ignores** (seven inherited plus the explicit
new image-options replay); **22 harness tests pass**. Original Go assertions
stay active. Direct option calls compare format, width, height and complete
error, without qualifying the other Options fields. CLI calls compare complete
stdout, terminal errors and file inventories/bytes/modes, including unchanged
sentinel files. Native requests contain inputs only. All **966 e2e rows**, full
CLI classifications and all 119 CLI-diff rows repeat; standard deltas show zero
regressions. Other full tiers are inherited; no upstream golden gain claimed.

Fresh inventory: **673 / 595 named / 78 without**; **Lane F 64 → 61**,
**CLI 36 → 33**. Next by output count: original
`TestIsometricImageMultiboardExports`, then `TestIsometricImageNativeCLI` and
the ordinary-PNG opt-out contract. Preserve the visible unsupported cyclic-tree
and mutable Go layout-option contracts rather than crediting subprocess coverage.

Evidence: **.reference/codex-runs/run88/** original captures, full negative
results, minimal witnesses, source mapping, **fix.patch**, guard reports/deltas,
**check.py**, cleanup and compression records; **report-run88.md**. Captures
have streaming **1 GiB total / 128 MiB file / 20 GiB free** guards and finite
deadlines. Registry adds **cli-network-api.json**, **cli-image-api.json** and
updates the current guards. No commits by this run; other changes and `29`
preserved. The reused warm test filename's earlier executable bindings are
explicitly retired in **native-runner-retirement.json**; their unrerun API
guards remain inherited, with all inputs and expected outputs preserved.

Deleted **three obsolete own staging files / 0.406 MiB allocated**, including
a source copy verified byte-identical to Git and a failed hook-compilation
attempt. No corpus inputs/expected outputs or shared caches retired; no bulk
moved. Applesauce **0.5.30**, verified LZFSE saves **70.926 MiB on 32 files**;
hashes, modes, owners and original non-compression xattrs verify, clone backups
were removed, and both executables still run. All **740 current + 622 previous
retained hashes verify**. Gross savings **71.332 MiB** are separate from net
checkout growth. Final disk audit: **run88/disk-final.json**; measurements and
the cleanup ledger are recorded in **report-run88.md**. Final allocated/apparent
GiB: checkout **22.73/40.09**, `.reference` **11.42/25.61**, `target`
**9.83/12.75**; **65.61 GiB free**. All ceilings pass. Net whole-checkout
allocated change is **+49.35 MiB**, distinct from the **71.33 MiB** gross
cleanup/compression savings. This run launches no further builds or parity
captures after the final audit.

### Lane L72: exhausted assignment; fresh regression guard (2026-10-25)

Re-read the complete brief through **Update 33**, newest L71/I55/F86
reports/messages and arriving I56 report. Updates **31–32 supersede Update
26**. Fresh inventory remains **673 added / 590 named / 83 without;
Lane L 109/111 observed**. Six residual source/type hashes and the original
residual record verify unchanged. Ordered by unsupported call count:
**TestNativeRasterAdmissionAndCancellation** has three Go typed-nil image
inputs; **TestNativeGIFLoopCloneAdmission** has two cyclic-scene inputs.
Native `Option<Rc<Texture>>` and owned `Vec<Node>` trees cannot represent
those inputs. Both functions remain visible and non-passing. No runtime
edits, builds, API recaptures, new observations or commits. Requested a new
actionable assignment; no reply received at handoff preparation.

Hash-verified **F86 executable** e2e guard: **933 SVG + 11 expected errors**,
all **966 classifications unchanged**, **fixed 0 / regressed 0** against
L53 and the shared baseline. This qualifies that retained executable, not
concurrent source. Other tiers inherited. Capture guard: **1 GiB total /
128 MiB file / 20 GiB free**, finite deadline. Evidence: **lang72/**;
handoff: **report-lang72.md**. Deleted this run's raw inventory and exact
duplicate report only: **475,136 allocated bytes / 0.453 MiB**; hashes,
replacements and reproduction instructions in **lang72/cleanup.json**.
Applesauce **0.5.30** read-only inspection; no compression writes/backups,
shared-cache/corpus retirement or bulk relocation. Final storage audit:
**lang72/disk-final.json**. Do not repeat the closed Lane L API sweeps.

Final allocated/apparent GiB: checkout **22.70/39.96**, `.reference` **11.39/25.57**, `target/` **9.83/12.66**; **69.05 GiB free**. All ceilings pass.
No builds or parity captures follow this audit.

### Lane L71: assignment unchanged; no actionable test cluster (2026-10-25)

Read the full brief through **Update 33**, latest L70/F86/I54 reports and
messages, arriving I55 report, and STATUS/PLAN/workflow. Updates **31–32**
supersede Update 26. Fresh inventory remains **673 added / 590 named /
83 without; Lane L 109/111 observed**. Six residual source/type hashes and
the original residual record verify unchanged. The three Go typed-nil image
calls in **TestNativeRasterAdmissionAndCancellation** and two cyclic-scene
calls in **TestNativeGIFLoopCloneAdmission** remain visible and non-passing.
No runtime edits, builds, API recaptures, observations or commits. A new
actionable assignment was requested; completed API sweeps were not repeated.

Required hash-bound **F86** executable e2e guard: **933 SVG + 11 expected
errors**, all **966 rows unchanged**, **fixed 0 / regressed 0**, against L53
and the shared baseline. This qualifies the retained executable, not concurrent
source. Other tiers inherited. Streaming **1 GiB total / 128 MiB file /
20 GiB free** guard and finite deadline applied. Evidence: **lang71/**;
handoff and final disk audit: **report-lang71.md**, **lang71/disk-final.json**.
Removed only this run's raw inventory and byte-identical duplicate report:
**475,136 allocated bytes / 0.453 MiB**, with hashes/replacements in
**lang71/cleanup.json**. No shared/corpus artifacts retired or bulk moved.

Final allocated/apparent GiB: checkout **22.69/39.95**, `.reference` **11.38/25.56**, `target/` **9.84/12.67**; **69.81 GiB free**. All ceilings pass
(**lang71/disk-final.json**, independent full scans; non-atomic during
concurrent work). No builds or parity captures follow this audit.

### Lane L70: exhausted assignment; F86 regression guard (2026-10-25)

Re-read the entire brief through **Update 33**, including its initial Update,
latest L69/F86/I54 reports/messages and STATUS/PLAN/workflow. Updates
**31–32 supersede Update 26**. Fresh inventory: **673 added / 590 named /
83 without; Lane L 109/111 observed**, unchanged. Six upstream/native source
hashes and the original residual record verify unchanged. The remaining
functions, **TestNativeRasterAdmissionAndCancellation** (three Go typed-nil
image calls) and **TestNativeGIFLoopCloneAdmission** (two cyclic-scene calls),
remain visible and non-passing. No runtime edits, builds, API recaptures,
new observations or commits. A new actionable assignment was requested;
do not repeat completed sweeps while the scope is unchanged.

Required hash-bound **F86** executable guard: **933 SVG + 11 expected
errors**, all **966 rows unchanged**, **fixed 0 / regressed 0** against L53
and the current shared baseline. This qualifies the retained executable,
not concurrent source. Other tiers inherited. Streaming **1 GiB total /
128 MiB file / 20 GiB free** guards and a finite deadline applied.
Evidence: **lang70/**; handoff: **report-lang70.md**. Removed only this
run's redundant raw inventory: **311,296 allocated bytes / 0.297 MiB**,
with path/hash/rebuild command in **lang70/cleanup.json**. No shared artifact
retired, compression writes/backups or bulk relocation. Final disk audit:
**lang70/disk-final.json** and **report-lang70.md**.

Final allocated/apparent GiB: checkout **22.58/39.88**, `.reference` **11.37/25.56**, `target/` **9.74/12.60**; **70.57 GiB free**. All ceilings pass;
scans include hidden paths/.git and every apparent hard link (non-atomic
during concurrent work). No builds or parity captures follow this audit.

### Run 87: watch authorization cookie/query parsing (2026-10-25)

Read the complete brief through **Update 33**, including its initial Update,
newest F86/I54/L69 reports/messages, and STATUS/PLAN/workflow. Continued Lane F.
All original watch-security assertions remain active. The two original test
functions already match on seven native calls; twelve separate source-derived
parser witnesses expose eight real failures.

| Gate | Before → after | Regressions |
|---|---:|---:|
| Original watch token/authorization calls | **7/7 unchanged** | **0** |
| Separate cookie/query/redirect witnesses | **4/12 → 12/12** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 unchanged** | **0** |
| Full CLI complete contracts / goldens, shared clock/release | **67 / 51 unchanged** | **0** |
| CLI diff, explicit shared clock | **119/119 unchanged** | **0** |
| Export / isometric mode calls | **16 / 29 unchanged** | **0** |

Source mapping: pinned master **6a7b48378**, `d2cli/watch.go:532–573` calls
Go **1.27.0** `Request.Cookie`, `URL.Query`, `URL.String` and `http.Redirect`.
Rust now selects the first successfully parsed cookie of the requested name,
including quoted values. It drops malformed escape/semicolon query pairs,
preserves arbitrary decoded query bytes, and decodes/re-escapes redirect paths
after clearing their original escape spelling. The witnesses cover duplicate
cookies, malformed neighbors/token values, encoded slashes and non-UTF-8 query
bytes. Complete application status/headers/body and authorization results
compare. Native socket length/close framing is checked separately from Go's
`ResponseRecorder`; random tokens compare error, length, URL safety and distinct
generation. Requests carry inputs only. No golden gain is claimed.

**249 CLI tests pass, seven ignores** (six existing plus the new explicit
watch replay); **five harness tests pass**. All **966 e2e classifications**
repeat; standard e2e/CLI deltas and all 119 CLI-diff rows have zero regressions.
The full CLI captures passed on both sides; the first scoring attempt lacked
Pillow, so the completed captures were rescored with `uv --with pillow --reuse`
without recapture or changing the comparison. Other full tiers are inherited.
Fresh inventory: **673 / 592 named / 81 without; Lane F 66 → 64,
CLI 38 → 36**. Next: the original private-network environment/help tests,
then remaining CLI image-option/admission contracts. Go mutable layout-option
snapshot APIs and the other lanes' scope decisions remain uncredited.

Evidence: **.reference/codex-runs/run87/** captures, complete negative baseline,
minimal pre-fix source, source mapping, guard reports/deltas and **check.py**.
Registry adds **cli-watch-api.json** and updates the corresponding current
guards. Captures have streaming **1 GiB total / 128 MiB file / 20 GiB free**
guards and finite deadlines. No commits; other changes and `29` preserved.
Cleanup/compression and final whole-checkout measurements are recorded in
**report-run87.md**, **run87/cleanup.json**, **run87/compression.json** and
**run87/disk-final.json**. No current corpus closure was retired or bulk moved.
Deleted **one superseded negative transport executable / 64.16 MiB allocated**;
its exact hash, minimal pre-fix source and complete negative results remain.
Verified Applesauce **0.5.30 / LZFSE saves 70.90 MiB on 32 files**; clone
backups removed after hash/mode/owner/xattr checks. Both compressed executables
still run; all **622 retained hashes verify**. Gross savings **135.07 MiB**
are distinct from whole-checkout net **+43.62 MiB allocated**. Final
allocated/apparent GiB: checkout **22.63/39.96**, `.reference` **11.39/25.58**,
`target` **9.76/12.66**; **69.05 GiB free**. All ceilings pass; six independent
full scans include hidden paths/.git and every apparent hard link, non-atomic
on the shared volume. No builds or parity captures follow the final audit.

### Lane L69: scope exhausted; latest qualified CLI guard (2026-10-25)

Read the full brief through **Update 33**, newest L68/I53/F85 reports and
messages, STATUS and PLAN/workflow. Updates **31–32 supersede Update 26**.
Fresh inventory: **673 added / 588 named / 85 without; Lane L 109/111**.
The six residual source/type hashes remain unchanged: three typed-nil image
calls in **TestNativeRasterAdmissionAndCancellation** and two cyclic-scene
calls in **TestNativeGIFLoopCloneAdmission** cannot be represented by native
`Option<Rc<Texture>>` / owned `Vec<Node>` trees. Both functions remain
visible and non-passing. No runtime edits, builds, observations or commits.
A new actionable failing test/package assignment was requested asynchronously;
no response received. Further unchanged API confirmation runs have no value.

Required e2e guard now uses the latest hash-verified **F85** executable:
**933 SVG + 11 expected errors**, all **966 classifications unchanged**,
**fixed 0 / regressed 0** against L53 and the shared baseline. This qualifies
the retained executable, not concurrent source. Other tiers inherited.
Evidence: **lang69/**; handoff: **report-lang69.md**. Streaming limits:
**1 GiB total / 128 MiB file / 20 GiB free**, finite deadline. Removed only
the redundant raw inventory: **311,296 allocated bytes / 0.297 MiB**;
exact path/hash/reproduction command in **lang69/cleanup.json**. No shared
artifacts retired. Final allocated/apparent GiB: checkout **22.58/39.88**,
`.reference` **11.37/25.55**, `target/` **9.74/12.60**; **70.57 GiB free**.
All ceilings pass (**lang69/disk-final.json**, six independent full scans;
non-atomic during concurrent work). No builds or parity captures follow.

### Run 86: CLI format selection and private SVG/text publication (2026-10-25)

Read the full brief through **Update 33**, including its initial Update,
latest F85/L66/I50 reports/messages and STATUS/PLAN/workflow. Continued Lane F.
The next original export contracts exposed a real failure: unknown-extension
SVG bytes matched Go, but new files had mode `0644` instead of `0600`.
Source-derived witnesses also exposed empty stdout-format rejection and
uppercase/alias filename suffixes selecting the wrong encoder.

| Gate | Before → after | Regressions |
|---|---:|---:|
| Original export CLI calls | **3/5 → 5/5** | **0** |
| Separate source-derived selection/replacement calls | **0/11 → 11/11** | **0** |
| Original isometric mode calls | **29/29 unchanged** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 unchanged** | **0** |
| Full CLI complete contracts / goldens, shared clock/release | **67 / 51 unchanged** | **0** |
| CLI diff, explicit shared clock | **119/119 unchanged** | **0** |

Source mapping, pinned master **6a7b48378**: `d2cli/export.go:43–66`
defaults an empty stdout flag to filename inference, which accepts only exact
supported extensions. The CLI now uses that rule independently of the library's
permissive parser. `main.go:1210–1219` and util-go v0.2.0
`xmain/xmain.go:185–216` publish SVG/text through a private temporary sibling,
checked close and rename, with the original direct-write fallback. Ordinary
SVG/text paths now do the same, including resetting an existing output's mode
when replacement succeeds. Raster/isometric writers keep their own contracts.

`cli-mode-scoreboard.py --suite export` retains both original tests' assertions
and separately names all eleven additional witnesses. Complete stdout, terminal
errors and file inventories/bytes/modes compare; no expected data enters native
requests. No output normalization. The explicit Go release stamp matches the
native release. **249 CLI tests pass, six existing ignores; eleven harness
tests pass.** All 966 e2e classifications and full CLI classifications are
unchanged under their standard delta scripts. Other full tiers are inherited;
no upstream golden gain is claimed.

Fresh inventory: **673 / 590 named / 83 without**; **Lane F 68 → 66**,
**CLI 40 → 38**. The Go `LayoutResolver` mutable pflag/seed-slice snapshot
tests lack an equivalent public mutable native API and remain uncredited.
Next: original private-network default/help and watch authorization contracts;
prioritize any exposed failure before widening. Other lanes' exhausted scope
handoffs are unchanged.

Evidence: **.reference/codex-runs/run86/** original captures, complete negative
baseline, final native replays, guard reports/deltas, source mapping,
**check.py**, cleanup/compression and disk records. Captures use streaming
**1 GiB total / 128 MiB file / 20 GiB free** guards and finite deadlines.
Current registry bindings include **cli-export-api.json**. No commits;
other changes and `29` preserved. Final disk/savings: **report-run86.md** and
**run86/disk-final.json**.

Deleted **75 files / 28.95 MiB allocated** (superseded own capture and idle F82
CLI test variant), with exact hash/rebuild records and an executable retirement
overlay. Verified Applesauce **0.5.30 / LZFSE saves 71.52 MiB / 36 files**;
backups removed after byte/mode/owner/xattr checks. Both compressed runners
execute. All **646 retained input/expected/native-output hashes verify**;
no current corpus closure or shared caches retired. Gross savings **100.48 MiB**
are distinct from whole-checkout net **+55.09 MiB allocated**. Final
allocated/apparent GiB: checkout **22.58/39.88**, `.reference` **11.37/25.55**,
`target` **9.74/12.60**; **70.57 GiB free**. All ceilings pass. No bulk moved,
and no builds/captures follow the completed final audit.

### Run 85: ELK option defaults and original isometric CLI contracts (2026-10-25)

Read the complete brief through **Update 33**, including its initial Update,
latest reports/messages and STATUS/PLAN/workflow. Continued Lane F's next
ELK-option queue, then batched original CLI contracts. **No runtime failure
was exposed; no runtime fix or golden gain is claimed.** Static comparison:
master **6a7b48378**, **d2layouts/d2elklayout/options_test.go:53–127**,
**layout.go:201–251**, and **d2cli/isometric_test.go:35–193**.

| Gate | Original calls / qualified result | Regressions |
|---|---:|---:|
| Three ELK option-constructor functions | **7/7** | **0** |
| Five original isometric CLI functions, release stamp | **29/29** | **0** |
| Six Dagre/ELK CLI spacing witnesses | **6/6 exact Go SVGs** | **0** |
| E2e SVG / expected errors, rebuilt CLI | **933 / 11 unchanged** | **0** |
| Full CLI complete contracts / goldens, shared clock/release | **67 / 51 unchanged** | **0** |

ELK checks default 50, explicit zero, and omission of parent spacing on leaf,
port and edge-label options. Only production change: extract the existing
port/inline-label option literals into functions used by both production and
replay. CLI checks mode precedence, implicit/default and source/flag SVG
parity, all nine failure calls, selected boards, and independent SVG/raster
scale limits. The original Go assertions remain active; native requests contain
only original inputs. Complete option JSON, stdout SVG bytes, terminal errors
and the entire file inventory/bytes/modes compare. **237 ELK tests + ten
harness tests pass**; the one ordinary ignored replay executes explicitly.

Ordinary Go source stamping initially differs on **18/29** CLI calls
(`v0.8.1-HEAD` vs native release `v0.9.0`); the full negative baseline remains
**run85/mode-before.json**. Qualification uses Go's explicit release link flag
`-X github.com/d2lang/d2/lib/version.Version=v0.9.0`. No output normalization or
runtime improvement is attributed to this profile change. All **966 e2e**
classifications and all full CLI classifications repeat under standard delta
scripts. Other full tiers remain inherited.

Fresh inventory: **673 / 588 named / 85 without**; **Lane F 76 → 68**,
**CLI 45 → 40**, **ELK options 3 → 0**. The six spacing witnesses do **not**
qualify the separate original `LayoutResolver` mutable-flag/seed-slice snapshot
functions: the native CLI owns its typed argument values and has no equivalent
public mutable xmain flag store. Assess that API applicability explicitly;
then batch remaining original CLI contracts that can expose a failure (stdout
format/unknown extension, private-network defaults and watch authorization).
Avoid repeating these already-matching option and mode cases.

Evidence: **.reference/codex-runs/run85/** final original captures/replays,
**elk-final.json**, **mode-final-release.json**, full guards/deltas,
**source.json**, **deltas.json**, **check.py**, cleanup/compression and disk
records. Registry: **elk-options-api.json**, **cli-mode-api.json**, plus current
e2e/CLI baselines. Every capture uses streaming **1 GiB total / 128 MiB file /
20 GiB free** guards and finite deadlines. Intermediate captures and the
superseded F84 CLI have explicit retirement/rebuild records; the full initial
source-clock negative result is preserved. No current input/expected-output
closure was retired. **All 540 retained input/expected/native-output files
verify original hashes.** No shared caches or other lanes' artifacts retired
(process inventory unavailable), no bulk moved, no commits; other changes and
`29` preserved. Final storage and savings: **run85/disk-final.json** and
**report-run85.md**.

Final allocated/apparent GiB: checkout **22.53/39.79**,
`.reference` **11.34/25.52**, `target` **9.71/12.55**;
**69.05 GiB free**. Every ceiling passes (**run85/disk-final.json**;
six independent full scans include hidden paths/.git and every apparent hard
link, non-atomic on the shared volume). Whole-checkout net change is
**+33.84 MiB allocated**, separately from the **127.14 MiB** operation
savings. No builds or parity captures follow this final audit.

### Run 84: Grid search threshold and fractional scoring (2026-10-25)

Read the full brief through **Update 33** (including the initial Update),
newest reports/messages and current STATUS/PLAN/workflow; continued Lane F.
The original **TestGridSearchMatchesReference** and
**TestGridIntegralSearchMatchesReference** exposed **five wrong partitions**.
Static source comparison: master **6a7b48378**,
**d2layouts/d2grid/layout.go:563–716,806–831** and
**grid_search.go:106–117**. Native search now keeps relaxing the threshold
until it finds a candidate and continues the source's bounded passes. It
uses Go's left-to-right size-plus-gap scoring and removes the arbitrary
equal-count fallback. Acceptance checks retain their separate addition order.

| Gate | Before → qualified | Regressions |
|---|---:|---:|
| Original search functions / calls | **0/2 → 2/2; 438/443 → 443/443** | **0** |
| Extracted nine-object CLI witness, Go SVG bytes | **0/3 → 3/3 engines** | **0** |
| E2e SVG / expected errors | **933 / 11 unchanged** | **0** |
| CLI complete contracts / goldens, release/shared clock | **67 / 51 unchanged** | **0** |

Witness: **tests/regressions/e2e/grid-search-threshold.d2**. The two original
Go tests keep their frozen-reference assertions; native requests contain
only dimensions, gaps, target size and object identities. Complete ordered
partitions compare across every original subtest, including 50/100/400-object
bounded searches. The five prior failures are ordinary native regression
fixtures. **103 layout tests + six harness tests pass**, with three ordinary
external replays ignored (this search replay executes explicitly). All **966
e2e classifications** and CLI classifications remain unchanged under the
standard delta scripts. No upstream golden gain; other full tiers inherited.
The new replay module passes rustfmt; checking the entire grid file also
reports the pre-existing Run 83 formatting at the zero-dimension error.

Fresh inventory: **673 / 580 named / 93 without**; **Lane F 77 → 76**
uncovered. Only the newly added integral-search function changes that count;
the fractional-search function predates master. This qualifies partitions,
not internal cache/allocation behavior. Next: assess the three remaining
ELK option contracts and CLI layout-option snapshots for an exposed failure;
avoid repeating completed or Go-only residual API sweeps.

Evidence: **.reference/codex-runs/run84/** search-before/final.json,
search-delta.json, source/, source.json, negative-grid-search.patch,
witness-{go,before,after}/, e2e/CLI reports/deltas, logs and check.py.
Registry **grid-search-api.json**, **e2e.json**, **cli-release-clock.json**
bind this run. Captures use streaming **1 GiB total / 128 MiB per file /
20 GiB free** limits and finite deadlines. Initial CLI scoring lacked Pillow;
the completed original captures were rescored with cached Pillow through
`uv run --no-project --with pillow --offline ... --reuse`.

All **876 retained capture/input/expected/native-output files** verify their
original hashes after cleanup/compression. Removed **nine obsolete files /
28.12 MiB allocated**, including the superseded F83 CLI snapshot; explicit
historical executable retirement/rebuild records are in cleanup.json.
Verified **Applesauce 0.5.30 / LZFSE** saves **46.47 MiB / 18 files**,
preserving bytes, executability, mode/owner and original non-compression
xattrs against clone backups, which were removed immediately. Gross savings
are **74.59 MiB**, separate from concurrent net checkout changes. No shared
caches or other lanes' artifacts were retired (`ps` denied), no corpus
input/output retired and no bulk moved. No commits; other lanes and `29`
preserved. Final disk audit: **run84/disk-final.json**; full handoff:
**report-run84.md**. Final allocated/apparent GiB: checkout **22.49/39.75**,
`.reference` **11.31/25.48**, `target` **9.71/12.55**; **68.28 GiB free**.
All ceilings pass. Whole-checkout net change is **+13.47 MiB allocated**
after builds/captures. Six independent full scans include hidden paths/.git
and every apparent hard link, non-atomic during concurrent lanes. No builds
or parity captures follow this audit.

### Run 83: Grid capacity admission and occupied storage (2026-10-25)

Read the complete brief through **Update 33**, newest F82/I43/L56 handoffs,
STATUS, PLAN and workflow; continued Lane F. The failing original functions
are **TestNewGridDiagramRejectsUnsafeCapacity** and
**TestGridCapacityBoundaryAndOccupiedStorage**. Static source comparison:
master **6a7b48378**, **d2layouts/d2grid/grid_diagram.go:31–131**,
**layout.go:233–337**, and **d2graph/grid_diagram.go:19–35**.

Native measured-Graph grid entry points now validate dimensions/object count
before multiplication/allocation, reuse the shared capacity bound and check
expanded dimensions. Even layout allocates only occupied row/column storage;
configured empty-cell gaps still contribute to the full diagram extent.
The 1000×1000 one-object witness retains **40060×40010** content dimensions
with **1×1** occupied storage. Both unsafe constructor witnesses now return
the complete original dimension-limit error. No solver heuristics changed.

| Gate | Before → qualified | Regressions |
|---|---:|---:|
| Original Grid functions / calls | **0/2 → 2/2; 2/5 → 5/5** | **0** |
| E2e SVG / expected errors | **933 / 11 unchanged** | **0** |
| CLI complete contracts / goldens, release/shared clock | **67 / 51 unchanged** | **0** |

**102 layout tests + six harness tests pass**. Two external replays remain
ignored in the ordinary layout suite; this run's replay executes explicitly.
Original Go assertions remain active; native replay requests contain only
inputs. The harness rejects incomplete inventories/results, changed errors,
wrong occupied dimensions, booleans in numeric results and leaked expectations.
All **966 e2e classifications** and every CLI classification remain unchanged
under the standard delta scripts. No golden gains; other full tiers inherited.

Fresh inventory: **673 / 579 named / 94 without**; **Lane F 79 → 77** uncovered.
This is finite typed positive-dimension Graph/API evidence, not Go string
parsing, arbitrary malformed graphs or full D2 parity. Next assess remaining
Grid integral/search and ELK option contracts, then CLI option snapshots;
do not repeat completed whole-board or other lanes' residual API captures.

Evidence: **.reference/codex-runs/run83/** capacity-baseline/qualified.json,
capacity-delta.json, source-qualified/, source.json, negative-grid.patch,
e2e/CLI reports and standard deltas, logs, cleanup/compression ledgers and
check.py. Registry: **grid-capacity-api.json**; e2e and release/shared-clock
CLI aliases now bind run83. Capture guards: **1 GiB total / 128 MiB file /
20 GiB free**, finite subprocess deadlines. Original hook was gofmt-formatted
and recaptured; all five original input/result calls repeat identically.
All **473 retained capture/input/expected/native-output files** verify original
hashes before/after cleanup. Historical baseline executable hashes are
explicitly retired; the negative patch and rebuild command preserve replay.
No commits; other lanes and `29` preserved. Removed **19 files / 27.62 MiB**;
verified Applesauce **0.5.30 / LZFSE** saves **44.93 MiB / 12 files**,
with byte/mode/owner/xattr checks and immediate backup removal. Exact paths,
replacement/rebuild records and rollback commands are in cleanup.json and
compression.json. Gross savings **72.56 MiB** are separate from
whole-checkout net **+139.52 MiB allocated** after builds/captures. No active or
hard-linked file rewritten or bulk moved. Final allocated/apparent GiB:
checkout **22.48/39.73**, `.reference` **11.29/25.47**,
`target` **9.71/12.55**; **70.95 GiB free**. Every ceiling passes;
six complete independent scans include hidden paths/.git and every apparent
hard link (**run83/disk-final.json**). No builds/captures follow that audit.
Full handoff: **report-run83.md**.

### Run 82: shared Dagre/ELK graph admission (2026-10-25)

Read the complete brief, including its initial Update and Updates through **33**,
newest reports/messages and STATUS; continued Lane F. Static comparison found
missing master admission in both native engines. Three original failures are
fixed: **TestLayoutRejectsDenseGraphBeforeDagre**, **TestLayoutRejectsDenseGraphBeforeELK**
and **TestLayoutChecksCancellationBeforeDagre**. ELK pre-cancellation already
matched. Source: master **6a7b48378**, **d2layouts/internal/layoutguard/graph.go:12–93**,
**d2dagrelayout/layout.go:48–63**, **d2elklayout/layout.go:252–269**.

One shared Rust guard checks cancellation, 1,024-object/edge limits, endpoint
presence and the 8,192 edge-interaction bound; graphs with at most 256 edges
retain the source fast path after endpoint checks. Sparse graphs and unrelated
objects follow the original accounting. All actual Dagre invocations, including
extracted parents/cells, now receive the caller token. Compiled empty boards
and specialized-layout boundaries retain their source behavior. No solver
geometry algorithm changes, fixture-specific outputs or error normalization.

| Gate | Before → final | Regressions |
|---|---:|---:|
| Original engine admission calls | **1/4 → 4/4** | **0** |
| Original shared-guard functions / calls | **new API: 8/8 / 14/14** | **0** |
| E2e SVG / expected errors | **933 / 11 unchanged** | **0** |
| CLI complete artifacts / goldens, release/shared clock | **67 / 51 unchanged** | **0** |

All **12 original functions / 18 calls** match, with original Go assertions
active and input-only native requests. Complete errors compare; Go error
identity remains a Go assertion. **520 native + five harness tests pass**
(CLI 249, core 69, layout 100, WASM 102); seven ordinary external replays remain
ignored, with this admission replay explicitly executed. All **966 e2e** and
all CLI classifications repeat under the standard delta scripts. No golden
gains; other full tiers remain inherited.

Requalified every retained layout-token input: **304 cancellation + 340 deadline**
cases. Exactly **55 historical Dagre errors** change under master. The current
Go probe uses active compilation and supplies the token at engine/router
callbacks, matching the existing native post-compilation boundary. This does
not qualify cancellation while compiling, the full master d2lib.Compile API,
or rebuilt browser/Node WASM packages. The old v0.9.0 fixtures and all original
inputs remain; the new fixture is source-observed, without weakened assertions.

Inventory at qualification: **673 / 576 named / 97 without**; **Lane F 91 → 79**
uncovered. This run adds 12 named functions; concurrent I41 accounts for 20.
Named observations are not full API parity. Next measure remaining Grid
capacity/integral and ELK option contracts, then the CLI option-snapshot queue.
Avoid repeating qualified whole-board captures or Lane L's pointer residuals.

Evidence: **.reference/codex-runs/run82/** admission-before/final.json,
context-fixture/retarget-delta.json, deltas.json, source.json/runtime.patch,
logs and verify.py; alias **layout-admission-api.json**. **1,933 new + 1,759
inherited input/expected-output bindings** verify, as do source restoration,
current runner hashes and compressed bytes/modes/owners. Captures have streaming
**1 GiB total / 128 MiB file / 20 GiB free** guards and finite deadlines.
No executable/source-tree freeze; no commits, other lanes and `29` preserved.

Confirmed cleanup retires **14 files / 270.45 MiB allocated path footprint**:
superseded Lane F CLI/test executables and reversible scoped-source staging.
Exact paths/hashes/replacements: **cleanup.json**, **cleanup-snapshots.json**,
**native-runner-retirement.json**. A shared Go-cache cleanup raced concurrent
activity and exited before persisting its in-memory deletion list. Its partial
inventory/byte count is unavailable and **excluded from savings**; see
**cache-cleanup-interrupted.json** and the failed log. Subsequent cleanup uses
a write-ahead ledger. No corpus inputs/expected outputs or canonical modules
were in that cache scope; their retained bindings verify.

Verified **Applesauce 0.5.30 / LZFSE saves 134.22 MiB on 21 retained files**,
including idle test runners and the frozen CLI. SHA-256, modes, owners and
original xattrs verify; clone backups removed. Exact paths and rollback:
**compression.json**, `applesauce decompress --verify -- <path>`. Local APFS
verified; DiskManagement unavailable. Confirmed savings **404.67 MiB** are
operation/path footprints, separate from concurrent net checkout changes.
No active/hard-linked/immutable object rewritten; no bulk moved.

Final allocated/apparent GiB: checkout **22.34/39.59**, `.reference` **11.16/25.34**, `target` **9.71/12.53**; **71.72 GiB free**. Every ceiling passes (**run82/disk-final.json**; six independent complete scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent lanes. No builds or parity captures follow this audit.

### Run 81: canceled formatting preserves the input file (2026-10-25)

Continued Lane F after reading the complete brief through **Update 33**,
newest reports/messages and STATUS. The original
**TestFmtHonorsCanceledContextBeforeWriting** exposed a real mismatch: Rust
formatted and rewrote `x:{y}\n` after cancellation; Go returned
`context canceled` without changing the file. `format_source` now forwards
the active token to the existing context-aware parser. Source: master
**6a7b48378**, **d2cli/fmt.go:42–53**, **d2parser/parse.go:52,251–261**.

The same capture also qualifies both original enabled-profile calls in
**TestEmptyDiagramRejectsUnavailableLayouts**, which already matched.
Original Go assertions stay active. Complete terminal errors, stdout, input
bytes/mode and output absence compare; native requests contain inputs only.
The observer uses upstream `xmain.mainFatal`/`cmdlog` and native
`write_cli_error` at the same boundary, without diagnostic normalization.
Initial observer setup/type/order/boundary issues are not runtime gains.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original admission calls, two functions | **2/3 → 3/3** | **0** |
| E2e SVG / expected errors | **933 / 11 unchanged** | **0** |
| CLI complete artifacts / goldens, release/shared clock | **67 / 51 unchanged** | **0** |

**249 native + five harness tests pass**; the normally ignored input transport
also executes explicitly. An ordinary native test covers canceled and expired
contexts, file bytes and mode. All **966 e2e classifications** and all CLI
case/golden classifications repeat, with standard zero-regression deltas.
No golden gains; other full tiers remain inherited. Go error identity,
disabled-engine build profiles and cancellation during filesystem writes are
outside this projection. No commits; other lanes and `29` preserved.

Fresh inventory: **673 added / 544 named / 129 without**, compared with
542 named at this run's start. **Lane F 93 → 91; CLI 47 → 45** uncovered.
Next: remaining original Dagre/ELK/TALA layout-option/snapshot functions and
layout admission contracts. Avoid repeating qualified board captures.

Evidence: **.reference/codex-runs/run81/** `admission-terminal-before.json`,
`admission-final.json`, `admission-qualified`, `deltas.json`, scoped source
hashes/runtime patch, test logs and `verify.py`; alias **cli-admission-api.json**.
All **1,759 new + 1,893 inherited input/expected-output bindings** verify,
including source restoration and compressed hashes/modes/owners. Old shared
runner hashes retire explicitly in **native-runner-retirement.json**.
No executable source freeze or full checkout copy. Captures have streaming
**1 GiB total / 128 MiB file / 20 GiB free** guards and finite deadlines.

Cleanup retires **4,389 files / 502.88 MiB allocated path footprint**: idle Go
action cache, superseded observer setup and the obsolete run80 CLI snapshot.
The snapshot's original hash and replacement/rebuild record remain in
**cleanup.json**; prior inputs/expected outputs remain. Other lanes' frozen
guards, current Cargo runners and source/probe reference tools are preserved.
Verified **Applesauce 0.5.30 / LZFSE saves 35.14 MiB on 16 retained files**.
Hashes, modes, owners and original xattrs match; clone backups are removed.
Exact targets/rollback: **compression.json**, `applesauce decompress --verify
-- <path>`. Local APFS verified; DiskManagement unavailable. No hard link,
active file or immutable object rewritten; no bulk moved. Savings are path
footprints, separate from concurrent whole-checkout changes.

Final allocated/apparent GiB: checkout **22.14/39.38**, `.reference`
**10.93/24.95**, `target` **9.74/12.71**; **72.48 GiB free**. Every ceiling
passes (**run81/disk-final.json**; six independent complete scans including
hidden paths/.git and every apparent hard link). Measurements are non-atomic
during concurrent lanes. No builds or parity captures follow this audit.

### Run 80: original paged admission and bounded document writer (2026-10-25)

Continued Lane F from run79 after reading the complete brief through **Update
33**, newest reports/messages and STATUS (including arriving I38/L40/L41).
Qualified **TestIsometricPagedAdmissionAndBoundedWriter: one original function /
nine calls**. Its original Go assertions remain active. Complete errors, all
four remaining page budgets, writer counts, remaining byte allowance and
emitted bytes match. Native requests contain inputs only.

Source: master **6a7b48378**, **d2cli/isometric_paged.go:135–175,215–235**.
Added the native bounded writer with cancellation-before-cap admission. The
production owned-document path shares that check and transfers its existing
allocation without a second buffer. An ordinary native test verifies exact-cap
acceptance, rejection, cancellation precedence and allocation preservation.
Page admission already matched; this adds a writer contract and observation,
**zero new golden matches**, rather than claiming a previously failing board.
The packager still builds its own document in memory; this is not a streaming
encoder or a cap on its internal allocations. Generic downstream partial-write
errors and Go's typed `errors.Is` identity are outside the result projection.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original paged function / calls | **new: 1/1 / 9/9** | **0** |
| E2e SVG / expected errors | **933 / 11 unchanged** | **0** |
| Paged isometric CLI source contracts | **24/24 unchanged** | **0** |
| CLI complete artifacts / goldens, release/shared clock | **67 / 51 unchanged** | **0** |

**248 native + five harness tests pass**; the normally ignored input transport
also executes explicitly. Every one of **966 e2e** and **24 paged CLI**
classifications repeats; the full CLI delta has no improvements or regressions.
Other full tiers are inherited. No commits; other lanes and `29` preserved.

Fresh inventory: **673 added / 522 named / 151 without**; **Lane F 94 → 93**,
**CLI 48 → 47**. Named observations are not proof of every upstream assertion.
Next prefer original cancellation/empty-diagram admission contracts or the
remaining layout-option functions; avoid repeating already-qualified board
artifacts and do not reopen Lane L's two Go-pointer residuals.

Evidence: **.reference/codex-runs/run80/** `paged-final`, input-only native
requests, `deltas.json`, scoped `source.json`, runtime patch, test logs and
`verify.py`; alias **paged-admission-api.json**. **1,893 new + 1,707 inherited
input/expected-output bindings** verify, as do Go source restoration and
compressed hashes/modes. Shared warm runner's previous executable hash is
explicitly retired in **native-runner-retirement.json**; original corpus
inputs/outputs remain. No executable source freeze or full checkout copy.
Captures use streaming **1 GiB total / 128 MiB file / 20 GiB free** guards
and finite deadlines.

Cleanup: **4,344 idle Go action-cache files / 420.21 MiB allocated path
footprint** removed after active-use checks; canonical module cache retained.
Two current Cargo executable variants stay; no stale rcgu objects were found.
Verified **Applesauce 0.5.30 / LZFSE saves 34.92 MiB** on retained evidence and
the idle CLI snapshot. Exact paths, hashes, replacements, preserved modes/
owners/xattrs and rollback: **run80/{cleanup,compression}.json**. Clone backups
removed; no hard link broken, active/shared executable compressed or bulk moved.
Local APFS verified; DiskManagement unavailable.

Final allocated/apparent GiB: checkout **21.93/39.17**, `.reference`
**10.72/24.74**, `target` **9.74/12.71**; **72.10 GiB free**. Every ceiling
passes (**run80/disk-final.json**; six independent complete scans, including
hidden paths/.git and every apparent hard link). Measurements are non-atomic
during concurrent work. No builds or parity captures follow the final audit.

### Lane I56: remaining URL ownership fix needs target scope (2026-10-25)

Read the full brief through **Update 33**, newest I55/L70/F86 reports and
messages, arriving L71 handoff, and STATUS/PLAN/workflow. Updates **30–32**
govern Lane I. Fresh inventory: **673 added / 590 named / 83 without;
Lane I 254/255**, unchanged. Only residual:
**TestIconURLIdentityAndOwnership**, pinned master **6a7b48378**,
**d2renderers/d2isometric/scene_test.go:200–215**. Static comparison with
**scene.go:182–200,329–343** confirms that isometric cannot recover private
credentials from the shared target's opaque JSON User field. Source/witness
hashes verify unchanged; all three prepared I46 patches still apply cleanly.
Requested the specific **d2-rs-target** scope extension asynchronously.
The reviewable fix remains **iso46/proposal.md**; no answer received during
this run. No runtime edits, builds, new API observations, coverage gains or
commits. Existing isolated proposal results remain draft evidence.

Required hash-bound **F86** executable e2e guard: **933 SVG + 11 expected
errors**, all **966 classifications unchanged**, **fixed 0 / regressed 0**
against F86 and the shared baseline. This qualifies the retained executable,
not concurrent source or the unapplied proposal. Other tiers inherited.
Streaming **1 GiB total / 128 MiB file / 20 GiB free** guard and a finite
deadline applied. Evidence: **iso56/** scope, proposal verification, e2e
execution/delta, cleanup and disk records; handoff: **report-iso56.md**.

Deleted only this run's byte-identical duplicate e2e report: **163,840
allocated bytes / 0.156 MiB**, hash and exact F86 replacement in
**iso56/cleanup.json**. Applesauce **0.5.30** read-only inspection confirms
that replacement already uses **LZFSE**. No compression writes/backups,
shared-cache/corpus retirement or bulk relocation. Other lanes and `29`
preserved. Final allocated/apparent GiB: checkout **22.76/40.02**,
`.reference` **11.40/25.57**, `target` **9.89/12.73**; **69.05 GiB free**.
All ceilings pass (**iso56/disk-final.json**, independent full scans including
hidden paths/.git and every apparent hard link; non-atomic during concurrent
work). `check.py verify` and STATUS whitespace validation pass. Do not repeat
completed API sweeps; integrate and
qualify the proposal after the scope decision, or assign new Lane I work.

### Lane I55: latest F86 guard; ownership scope remains pending (2026-10-25)

Read the entire brief through **Update 33**, including the initial Update,
newest I54/L69/F86 reports/messages, arriving L70 report, and STATUS/PLAN/workflow.
Updates **30–32** govern this lane. Fresh inventory remains **673 added /
590 named / 83 without; Lane I 254/255**. The only residual is
**TestIconURLIdentityAndOwnership**, pinned master **6a7b48378**,
**d2renderers/d2isometric/scene_test.go:200–215**. Static comparison with
**scene.go:182–200,329–343** confirms the shared URL credentials/ownership gap.
Source/witness hashes remain unchanged and all three I46 proposal patches
pass `git apply --check`. Integration requires **d2-rs-target**, outside this
lane's explicit boundary. Requested that narrow extension asynchronously;
no response received. The reviewable fix remains **iso46/proposal.md**.
No production edits, builds, new API observations, coverage gains or commits.

Fresh hash-bound **F86** executable guard: **933 SVG matches + 11 expected
errors**, **966 unchanged classifications**, **fixed 0 / regressed 0** against
F86 and the shared baseline. This qualifies the retained executable, not
current concurrent source or the unapplied proposal. Other tiers inherited.
Streaming **1 GiB total / 128 MiB file / 20 GiB free** limits and a finite
deadline applied. One initial wrapper launch used the wrong working directory
and failed before producing a report; the corrected fresh launch passes.
Both launch records remain explicit in **.reference/codex-runs/iso55/**.

Deleted only this run's byte-identical duplicate e2e report: **163,840
allocated bytes / 0.156 MiB**, exact hash and F86 replacement in **cleanup.json**.
Applesauce **0.5.30** read-only inspection confirms the retained F86 report
already uses **LZFSE**. No compression writes/backups, shared-cache/corpus
retirement or bulk relocation. Other lanes' changes and `29` preserved.
Final allocated/apparent GiB: checkout **22.60/39.89**, `.reference`
**11.38/25.56**, `target` **9.74/12.61**; **69.81 GiB free**. All ceilings
pass (**iso55/disk-final.json**; six independent full scans include hidden
paths/.git and every apparent hard link, non-atomic during concurrent work).
`check.py verify` passes; no builds or parity captures follow this audit.
Full handoff: **report-iso55.md**. The next useful action is the scope decision and
integrated qualification; completed API confirmation sweeps cannot advance
this assignment.

### Lane I54: ownership integration remains outside scope (2026-10-25)

Read the full brief through **Update 33**, including its initial Update,
newest I53/L68/F85 reports/messages and current STATUS/PLAN/workflow. Updates
30–32 supersede 27. Fresh inventory: **673 added / 590 named / 83 without**;
**Lane I remains 254/255 named**. The sole residual remains
**TestIconURLIdentityAndOwnership**, pinned master **6a7b48378**,
**d2renderers/d2isometric/scene_test.go:200–215**. Static source comparison:
**scene.go:182–200,329–343**. Existing I46 source/witness/proposal hashes
verify and all three patches pass `git apply --check`. The complete,
reviewable fix is **iso46/proposal.md**; its required **d2-rs-target** edit
is outside Lane I's assigned boundary. Requested that specific extension
asynchronously; no answer received. No production edits, builds, new API
captures, coverage gains or commits; I46 draft results remain draft evidence.

Required retained-F79 executable e2e guard: **933 SVG + 11 expected errors**,
all **966 classifications unchanged**, **fixed 0 / regressed 0**, against
I42 and the current shared baseline. Runner hashes verify before/after;
this does not qualify current source or the unapplied proposal. Streaming
**1 GiB total / 128 MiB file / 20 GiB free** guards and a finite deadline
applied. Evidence: **.reference/codex-runs/iso54/**. Removed only this run's
byte-identical duplicate report: **0.156 MiB allocated**; original hash and
exact replacement in **cleanup.json**. Applesauce **0.5.30** read-only
inspection confirms retained I42 report already uses **LZFSE**; no compression
writes/backups, shared-cache retirement or bulk relocation. Other lanes and
`29` preserved. Final allocated/apparent GiB: checkout **22.58/39.88**,
`.reference` **11.37/25.55**, `target` **9.74/12.60**; **70.57 GiB free**.
All ceilings pass (**iso54/disk-final.json**; independent full scans include
hidden paths/.git and all apparent hard links, non-atomic during concurrent
work). The first final scan exited nonzero; its fresh retry and `check.py
verify` pass. No builds or parity captures follow the final audit.
Full handoff: **report-iso54.md**.
The next useful action remains the scope decision and integrated qualification;
unchanged API confirmation runs cannot advance this assignment.

### Lane I53: residual ownership fix awaits scope extension (2026-10-25)

Read the entire brief through **Update 33**, newest I52/L68/F85 reports and
messages, STATUS/PLAN/workflow. Fresh inventory remains **673 added / 588
named / 85 without; Lane I 254/255 named**. Sole residual:
**TestIconURLIdentityAndOwnership**, pinned master
**d2renderers/d2isometric/scene_test.go:200–215**. Current upstream/native,
original witness and I46 proposal hashes verify; all three patches still
pass `git apply --check`. The required **d2-rs-target** change remains outside
Lane I's edit boundary. Scope extension requested asynchronously; no answer
received. No production edits, builds, API recaptures, coverage gains or commits.

Required retained-F79 e2e guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, **fixed 0 / regressed 0** against I42 and the
shared baseline. Runner hashes verify before/after; this qualifies that
executable, not current source or the unapplied proposal. Evidence:
**.reference/codex-runs/iso53/**. Streaming **1 GiB total / 128 MiB file /
20 GiB free** guards and a finite deadline applied. Removed only the
byte-identical duplicate report: **0.156 MiB allocated**, exact hash and
replacement in **cleanup.json**. Applesauce **0.5.30** read-only inspection
confirms retained I42 evidence already uses **LZFSE**; no compression writes,
shared-cache retirement or bulk relocation. Other lanes and `29` preserved.
Final allocated/apparent GiB: checkout **22.68/39.94**, `.reference` **11.37/25.55**, `target` **9.83/12.67**; **70.57 GiB free**. All ceilings
pass (**iso53/disk-final.json**; independent scans include hidden paths/.git
and every apparent hard link, non-atomic during concurrent work). No builds
or parity captures follow this audit.
The next useful action is the scope decision, followed by integration and
fresh target/isometric/whole-board guards; do not repeat unchanged API sweeps.

### Lane I52: scope decision remains the next action (2026-10-25)

Read the full brief through **Update 33**, latest I51/L66/F85 reports and
messages, STATUS/PLAN/workflow. Updates **30–32 supersede 27**. Fresh inventory:
**673 added / 588 named / 85 without**; Lane I remains **254/255 named**.
Only residual: **TestIconURLIdentityAndOwnership**, master **6a7b48378**,
**d2renderers/d2isometric/scene_test.go:200–215**. The existing
**iso46/proposal.md** and three patches still verify against source/capture
hashes and `git apply --check`. Integration needs **d2-rs-target**, outside
the lane boundary; explicit scope extension requested, no answer received.
No production edits, builds, API recaptures, new coverage or commits.
Next action is that scope decision or a new assignment; another unchanged
API sweep would not advance the port.

Required retained-F79 e2e guard: **933 SVG + 11 expected errors**; all **966
classifications unchanged**, **fixed 0 / regressed 0** against both retained
I42 and current shared baseline. Runner hash verifies before/after. This
qualifies the retained binary, not current source or the unapplied proposal.
Evidence: **.reference/codex-runs/iso52/** scope/proposal verification,
e2e-run/delta, cleanup and fresh disk JSON; `check.py verify` validates them.
Streaming **1 GiB total / 128 MiB file / 20 GiB free** guards and finite
deadlines applied. Removed only this run's byte-identical duplicate report:
**163,840 allocated bytes / 0.156 MiB**; original hash and exact replacement
are in **cleanup.json**. Applesauce **0.5.30** read-only inspection confirms
retained I42 evidence already uses **LZFSE**; no compression writes/backups,
shared-cache retirement or bulk relocation. Other lanes and `29` preserved.
Final allocated/apparent GiB: checkout **22.54/39.80**, `.reference` **11.35/25.53**, `target` **9.71/12.55**; **69.43 GiB free**. All ceilings pass (**iso52/disk-final.json**; independent whole scans include hidden paths/.git and every apparent hard link, non-atomic during concurrent work). No builds or parity captures follow the audit.

### Lane I51: exhausted scope; ownership integration still pending (2026-10-25)

Read the entire brief through **Update 33**, including its initial Update,
latest I50/L65/F84 reports/messages and arriving L66/F85, STATUS/PLAN/workflow.
Fresh inventory: **673 added / 588 named / 85 without**; Lane I remains
**254/255 named**. Sole residual: **TestIconURLIdentityAndOwnership**,
master **6a7b48378**, **d2renderers/d2isometric/scene_test.go:200–215**.
I46's proposal/source/capture hashes and all three patch applicability checks
verify unchanged. Integration requires **d2-rs-target**, outside Lane I's
edit boundary. Explicit extension requested asynchronously; no reply received.
No runtime edits, builds, API recaptures, new observations or commits.
Further unchanged-scope handoffs cannot advance this assignment; integrate
**iso46/proposal.md** after a scope extension or provide a new failing test.

Mandatory retained-F79 e2e guard: **933 SVG + 11 expected errors**, all
**966 classifications unchanged**, **fixed 0 / regressed 0** against I42 and
the current shared baseline. Runner hash verifies before/after. This checks
the retained executable, not concurrent source or the unapplied draft.
Streaming **1 GiB total / 128 MiB file / 20 GiB free** guards and finite
deadlines apply. Other full tiers inherited. Evidence: **iso51/**; audit
reuses hash-bound I50 code rather than duplicating another API sweep.

Deleted only I51's duplicate e2e report after exact equality/hash checks
against retained I42: **163,840 allocated bytes / 0.156 MiB**;
**cleanup.json** records the path/hash/replacement. Applesauce **0.5.30**
read-only inspection confirms the replacement already uses **LZFSE**.
No shared-cache/corpus retirement, compression writes/backups or bulk movement.
Other lanes and stray `29` preserved.

Final allocated/apparent GiB: checkout **22.53/39.79**, `.reference` **11.34/25.52**, `target` **9.71/12.55**; **69.81 GiB free**. All ceilings pass (**iso51/disk-final.json**; independent full scans include hidden paths/.git and every apparent hard link, non-atomic during concurrent work). No builds or parity captures follow this audit.

### Lane I50: remaining ownership fix requires scope decision (2026-10-25)

Read the complete brief through **Update 33**, newest I49/L64/F84 reports
and messages, arriving L65, and STATUS/PLAN/workflow. Updates **30–32**
supersede 27. Fresh inventory: **673 added / 580 named / 93 without**;
Lane I remains **254/255 named**, with only
**TestIconURLIdentityAndOwnership** uncovered (master **6a7b48378**,
**d2renderers/d2isometric/scene_test.go:200–215**).

I46's prepared proposal, production/source/capture hashes and patch
applicability verify unchanged. Integration requires **d2-rs-target**,
outside Lane I's assigned edit boundary. Requested the explicit scope
extension asynchronously; no answer received. No runtime edits, builds,
repeated API captures, coverage gains or commits. The isolated I46 results
remain draft evidence. Next action is a scope decision on
**.reference/codex-runs/iso46/proposal.md** or a new failing-test assignment;
further unchanged-scope confirmation runs cannot advance implementation.

Required retained-F79 e2e guard: **933 SVG + 11 expected errors**, all
**966 classifications unchanged**, **fixed 0 / regressed 0** against I42
and the shared baseline. Runner SHA-256 verifies before/after. This checks
the retained executable, not current concurrent source or the unapplied
proposal. Other full tiers inherited. Streaming **1 GiB total / 128 MiB
file / 20 GiB free** limits and finite deadlines apply.

Evidence: **iso50/** scope/proposal-verification, e2e-run/delta/cleanup/disk
JSON, logs and **check.py**; **report-iso50.md**. Deleted only this run's
duplicate e2e report after byte/hash verification against retained I42:
**163,840 allocated bytes / 0.156 MiB**. No corpus/shared-cache retirement,
compression writes, backups or bulk relocation. Other lanes and `29`
preserved. Final storage measurements: **iso50/disk-final.json**.

Final allocated/apparent GiB: checkout **22.65/39.85**, `.reference` **11.35/25.53**, `target` **9.82/12.60**; **69.05 GiB free**. All ceilings pass (**iso50/disk-final.json**; six independent full scans include hidden paths/.git and every apparent hard link, non-atomic during concurrent work). No builds or parity captures follow this audit.

### Lane I49: existing ownership proposal awaits scope extension (2026-10-25)

Read the complete brief through **Update 33**, latest I48/F84/L63 reports
and messages, arriving L64, STATUS, PLAN and workflow. Fresh inventory: **673 added /
580 named / 93 without**; Lane I remains **254/255 named**. Only residual:
**TestIconURLIdentityAndOwnership**, pinned **6a7b48378**, **scene_test.go:200–215**.
I46's complete proposal, source/capture hashes and patch applicability verify
unchanged. Integration needs **d2-rs-target**, outside Lane I's assigned edit
boundary. Scope extension requested asynchronously; no reply received.
No production edits, builds, repeat API captures, coverage gains or commits.
Next action is the existing proposal's scope decision or a new failing-test
assignment; unchanged-scope confirmation runs cannot advance implementation.

Required retained-F79 e2e guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, **fixed 0 / regressed 0** against I42 and the
current shared baseline. Runner hash verifies before/after. This qualifies
the retained executable only; I46's proposal remains unapplied. Other tiers
inherited. Streaming **1 GiB total / 128 MiB file / 20 GiB free** guards and
finite deadlines apply. Evidence: **iso49/** scope/proposal-verification,
delta/e2e-run/cleanup/disk JSON, logs and **check.py**; **report-iso49.md**.
Deleted only this run's duplicate e2e report after byte/hash verification
against retained I42: **163,840 allocated bytes / 0.156 MiB**. Applesauce
**0.5.30** read-only inspection confirms that replacement already uses LZFSE.
No compression writes, shared-cache/corpus retirement or bulk movement.
Other lanes and `29` preserved. Disk audit: **iso49/disk-final.json**.

Final allocated/apparent GiB: checkout **22.55/39.81**, `.reference` **11.31/25.49**, `target` **9.77/12.60**; **67.14 GiB free**. All ceilings pass (**iso49/disk-final.json**). Six independent full scans include hidden paths/.git and every apparent hard link; non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane I48: assignment remains blocked at shared target ownership (2026-10-25)

Read the complete brief through **Update 33**, latest I47/L62/F83 reports and
messages, arriving L63, STATUS, PLAN and workflow. Updates **30–32** supersede
27. Fresh inventory: **673 added / 580 named / 93 without**; Lane I remains
**254/255 named**. The sole residual is **TestIconURLIdentityAndOwnership**,
master **6a7b48378**, **d2renderers/d2isometric/scene_test.go:200–215**.

I46 already prepared and isolated-tested the complete proposal. Integration
requires **d2-rs-target**, outside the brief's Lane I edit boundary. No scope
extension has arrived. No production changes, builds, repeated API sweeps,
new observations or commits. Next action remains a target-crate scope extension
for the existing proposal or a new failing-test assignment; another unchanged
scope run cannot advance implementation. Draft qualification is inherited,
not repeated or promoted to production evidence.

Mandatory retained-F79 guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, **fixed 0 / regressed 0**, against I42 and the
current shared baseline. Runner SHA-256 verifies before/after. This checks
the retained executable, not concurrent sources or the unapplied proposal.
Other tiers inherited. Streaming **1 GiB total / 128 MiB file / 20 GiB free**
guards and finite deadlines apply. Evidence: **iso48/{scope,delta,e2e-run,
cleanup,disk-initial,disk-final}.json**, logs and **check.py**; **report-iso48.md**.

Deleted only this run's byte-identical duplicate e2e report after verifying
its retained I42 replacement: **163,840 allocated bytes / 0.156 MiB**.
No corpus inputs/outputs, shared caches, proposals or other lanes' files
retired. No compression writes, backups or bulk movement. Only tracked edit:
this STATUS section.

Final allocated/apparent GiB: checkout **22.49/39.75**, `.reference` **11.31/25.48**, `target` **9.71/12.55**; **67.90 GiB free**. All ceilings pass (**iso48/disk-final.json**). Six independent complete scans include hidden paths/.git and all apparent hard links; non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane I47: scope handoff; one target ownership contract remains (2026-10-25)

Read the full brief, including its initial Update through **33**, newest
I46/L61/F83 reports/messages, STATUS/PLAN/workflow and arriving L62 handoff.
**Updates 30–32 supersede 27.** Fresh inventory: **673 / 579 named / 94 without**;
Lane I stays **254/255 named**. The only uncovered original function remains
**TestIconURLIdentityAndOwnership**, master **6a7b48378**, **scene_test.go:200–215**.
Static comparison confirms that an isometric-only adapter cannot resolve the
shared typed target's missing credential ownership. The complete I46 proposal,
source/capture hashes and clean patch applicability verify unchanged.

Requested explicit **d2-rs-target** scope extension asynchronously; no answer
received. Current lane boundaries exclude that crate. No production edits,
builds, API recaptures, new observations or commits. I46's isolated results
remain draft evidence. Next authorize integration of that existing proposal
or assign a new failing-test queue; avoid repeated exhausted-scope runs.

Required retained-F79 guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, **fixed 0 / regressed 0**, against I42 and the
current shared baseline. Runner hash verifies before/after. This checks the
retained executable, not concurrent changes or the unapplied proposal.
Other full tiers inherited. Streaming **1 GiB total / 128 MiB file / 20 GiB
free** guards and finite deadlines apply. Evidence: **iso47/** scope/delta/
e2e-run/cleanup/disk JSON, logs and **check.py**; **report-iso47.md**.

Removed this run's duplicate report and raw inventory: **471,040 allocated
bytes / 0.449 MiB**; exact paths, hashes and replacements in **cleanup.json**.
Original I23 input/output closure verifies through I46 checks. Read-only
**Applesauce 0.5.30** confirms retained I42 report already uses LZFSE
(161,836 logical / 16,384 compressed bytes). No compression rewrite/backup,
shared-cache/corpus retirement or bulk movement. Other lanes and `29` preserved.
Only supervisor tracked edit: this I47 section. This is a scope handoff, not
implementation progress.

Final allocated/apparent GiB: checkout **22.44/39.69**, `.reference` **11.31/25.48**, `target` **9.66/12.49**; **68.28 GiB free**. All ceilings pass (**iso47/disk-final.json**). Six independent complete scans include hidden paths/.git and every apparent hard link; non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane I46: complete and test the unapplied URL-ownership fix (2026-10-25)

Read the full brief, including its initial Update through **33**, newest
reports/messages and STATUS/PLAN/workflow; also read arriving F83/L61 handoffs.
**Updates 30–32 supersede 27**. Fresh inventory: **673 added / 579 named /
94 without**; Lane I remains **254/255 named**, one uncovered:
**TestIconURLIdentityAndOwnership**. No production changes, new Go captures,
inventory credit or commits. Other lanes and `29` preserved.

Completed I45's **unapplied** proposal in **.reference/codex-runs/iso46/**.
The third patch, **proposed-admission.patch**, visits icon owners and private
username/password bytes in Go declaration order, including root/legend icons
and cumulative collection/text admission. It preserves competing errors before
and after the icon field. Explicit exhaustive owner destructuring catches
future target-field additions at compilation. Sources: master **6a7b48378**,
**scene_test.go:200–215; scene.go:136–146,182–200,329–390**,
**d2target.go:103–106,769–825,898–941**; Go 1.27.0
**net/url/url.go:276–366**, **gen_encoding_table.go:148–235**.

All three patches parse and pass `git apply --check`. An evidence-local build
of the **actual scene builder** plus proposed target/admission code passes
**13 tests**, including ownership/mutation, UTF-8 credential-byte accounting,
optional/empty passwords, escaping, opaque JSON and failure precedence.
The isolated patched runner replays the complete retained I23 corpus:
**8 functions / 26 operations match**, including the original credential case;
**zero draft regressions** from seven matching functions plus one unsupported.
Original source/assertion/capture hashes verify; expected scenes never supply
native inputs. **Draft results do not qualify integrated production code.**

Requested explicit **d2-rs-target** scope extension asynchronously; none received.
The current brief excludes that crate from Lane I's edit boundary. The patches
remain unapplied. After authorization: integrate, update provenance, build/test
target/isometric crates and run whole-board scene/SVG/e2e on the new executable.
Full workspace/downstream callers and whole-board rendering through this change
remain unqualified. No repeat of completed API sweeps is warranted.

Required frozen-F79 e2e guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, **fixed 0 / regressed 0**, also matching F83's
current baseline. Runner hash verifies before/after. This guards existing code;
other full tiers inherited. Captures/builds have streaming **1 GiB total /
128 MiB file / 20 GiB free** limits and finite deadlines. Evidence:
**iso46/{proposal.md,source.json,draft-validation.json,draft-replay-validation.json,
scope.json,delta.json,cleanup.json}**, patches, scripts/logs and **report-iso46.md**.
Verification: `uv run --no-project python .reference/codex-runs/iso46/check.py verify`
with the shared UV cache. Supervisor tracked path: **this I46 STATUS section only**.

Deleted **11 own disposable paths / 5,779,456 allocated bytes (5.512 MiB)**:
draft sources/binaries/request/successful output and redundant report/inventory.
Hashes/rebuild commands are in **cleanup.json**; retained I23 input/output closure
verifies. Applesauce **0.5.30** confirms the retained e2e baseline already uses
LZFSE. No compression rewrite, backup, shared-cache/corpus retirement or bulk
movement. Process inspection is denied; shared artifacts left alone. Final
whole-checkout audit: **iso46/disk-final.json**.

Final allocated/apparent GiB: checkout **22.48/39.74**, `.reference` **11.29/25.47**, `target` **9.71/12.55**; **65.99 GiB free**. All ceilings pass. Six independent complete scans include hidden paths/.git and all apparent hard links; non-atomic during concurrent lanes. No builds or parity captures follow this audit.

### Lane I45: concrete unapplied Userinfo proposal (2026-10-25)

Read the complete brief, including the initial Update and all Updates through
**33**, newest reports/messages and STATUS/PLAN/workflow. **Updates 30–32
supersede 27**. Fresh inventory: **673 added / 577 named / 96 without**;
Lane I remains **254/255 named**, with **TestIconURLIdentityAndOwnership**
uncovered. Other lanes and `29` preserved; no commits or production changes.

Prepared reviewable, **unapplied** patches in **.reference/codex-runs/iso45/**:
**proposed-userinfo.patch** adds owned credentials with Go-compatible `{}` JSON,
optional-password semantics and URL escaping; **proposed-scene-replay.patch**
uses the retained source-URL sidecar and adds a native ownership test. Both
pass rustfmt parsing and `git apply --check`; **neither compiled nor executed**.
Source: master **6a7b48378**, **scene_test.go:200–215**,
**scene.go:182–200,329–343**, pinned Go 1.27.0 **net/url/url.go:309–366** and
**gen_encoding_table.go:148–235**. No new API passing claim.

Requested explicit target-crate scope extension asynchronously; no reply or
authorization assumed. The brief excludes **d2-rs-target** from Lane I's edit
scope. **proposal.md** records the exact follow-up: also count private credential
bytes in scene admission, preserving Go's failure order (**scene.go:345–390**);
the current serializer-based budget cannot see them. Complete that work, then
compile/replay the retained original capture and run fresh whole-board guards.
Avoid another completed-API capture sweep while the scope decision is pending.

Required frozen-CLI guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, standard delta **fixed 0 / regressed 0**.
Runner hash verifies before/after; this does not validate the unapplied proposal.
Other full tiers inherited. Evidence: **report-iso45.md**,
**iso45/{scope,source,proposal-validation,delta,e2e-run,cleanup}.json**, patches,
**check.py**, **prepare.py** and logs. Streaming **1 GiB / 128 MiB file /
20 GiB free** limits and finite deadline. No builds or new Go/API captures.

Deleted this run's redundant report/inventory after verification: **471,040
allocated bytes / 0.449 MiB**. Exact hashes/replacements: **cleanup.json**.
Retained I42 report already uses **LZFSE**, verified read-only with **Applesauce
0.5.30**. No compression writes/backups/shared retirement or bulk movement.
Process inspection is denied; no shared cleanup attempted. Supervisor tracked
path: **this I45 STATUS section only**. Final audit: **iso45/disk-final.json**.

Final allocated/apparent GiB: checkout **22.42/39.67**, `.reference` **11.22/25.40**, `target` **9.72/12.55**; **71.33 GiB free**. All ceilings pass. Six independent complete scans include hidden paths/.git and all apparent hard links; non-atomic during concurrent lanes. No builds or parity captures follow the final audit.

### Lane I44: remaining URL/Userinfo scope boundary (2026-10-25)

Read the complete brief through **Update 33**, newest I43/L56/F82 reports and
last messages, and current STATUS/PLAN/workflow. Updates **30–32 supersede 27**.
Fresh inventory remains **673 added / 577 named / 96 without**; Lane I remains
**254/255 named, one uncovered: TestIconURLIdentityAndOwnership**.

Original I23 witness and upstream/restored trace/native hashes verify.
Master **6a7b48378**, **scene_test.go:200–215; scene.go:182–200,329–343**
preserves and owns private URL credentials; native **d2-rs-target/src/target.rs:
357–379,481–505** cannot retain those inputs. The next fix needs an owned
Userinfo API in the target crate, outside current Lane I scope. Requested that
scope extension asynchronously; no reply/authorization assumed. The test stays
uncovered and non-passing. No production edits, builds, new API captures,
observations, exclusions or commits; no implementation or full-parity gain.

Required e2e guard: **933 SVG + 11 expected errors**, **966 unchanged
classifications**, standard delta **fixed 0 / regressed 0**. Frozen F79 CLI
hash verifies before/after against I42. Other full tiers remain inherited.
Evidence: **.reference/codex-runs/report-iso44.md**, **iso44/{scope,source,
delta,e2e-run,cleanup}.json**, **check.py** and logs. Streaming **1 GiB total /
128 MiB file / 20 GiB free** limits and finite deadline.

Removed only this run's duplicate report/inventory: **471,040 allocated bytes
(0.449 MiB)**, with exact hashes/replacements/reproduction in **cleanup.json**.
The e2e report was byte-identical to retained I42 evidence, already **LZFSE**
compressed per read-only **Applesauce 0.5.30** inspection. No compression writes,
backups, shared cache mutation or corpus retirement; no bulk moved. Other lanes
and `29` preserved. Supervisor tracked scope: this I44 section only.
Final storage evidence: **iso44/disk-final.json**.

Final allocated/apparent GiB: checkout **22.34/39.59**, `.reference` **11.17/25.35**, `target` **9.70/12.52**; **71.72 GiB free**. All ceilings pass. Six independent complete scans include hidden paths/.git and every apparent hard link; measurements are non-atomic during concurrent work. No builds or parity captures follow the final audit.

### Lane I43: scoped URL/Userinfo handoff; no new capture (2026-10-25)

Read the full brief including its initial Update and Updates through **33**,
newest I42/L55/F81 reports/messages and current STATUS; also read arriving
F82/L56 handoffs (no scope change). **Updates 30–32
supersede Update 27.** Fresh inventory: **673 added / 577 named / 96 without**.
Lane I remains **254/255 named, one uncovered** (the current split plus scene
and imageasset): **TestIconURLIdentityAndOwnership**. I42 completed the final
image function; no completed capture was repeated.

Confirmed the unchanged input boundary against pinned **6a7b48378**:
**scene_test.go:200–215**, **scene.go:182–200,329–343** explicitly retain and
clone URL credentials; **d2-rs-target/src/target.rs:357–379,481–505** retains
opaque JSON `User: {}` only. The original I23 credential-bearing witness and
upstream/restored trace source hashes verify. This needs an owned URL/Userinfo
API in the target crate, outside current Lane I scope. Requested an extension
or reassignment asynchronously; no answer/approval is assumed. The test stays
uncovered and non-passing. No actionable in-scope failing board was identified.

Required e2e guard: **933 SVG + 11 expected errors**, **966 unchanged
classifications**, standard delta **fixed 0 / regressed 0**. The unchanged
frozen F79 CLI hash verifies against I42 before/after. No runtime edits,
builds, new API captures/observations, exclusions or commits; other full tiers
remain inherited. This is a handoff, not implementation or full-parity progress.
Supervisor tracked path: **this I43 section only**; other lanes and `29` preserved.

Evidence: **.reference/codex-runs/report-iso43.md**, **iso43/{scope,source,
delta,e2e-run,cleanup}.json**, **check.py**, e2e and standard delta logs.
Streaming **1 GiB total / 128 MiB file / 20 GiB free** guard and finite deadline.
Deleted only this run's duplicate report/inventory: **471,040 allocated bytes
(0.449 MiB)**, with hashes, replacements and reproduction commands in
**cleanup.json**. Byte-identical e2e replacement is I42's report; Applesauce
**0.5.30** confirms it is already **LZFSE compressed**. No recompression,
backup, shared cache mutation or corpus retirement; new e2e log is 497 bytes.
Fresh six-scan final storage evidence: **iso43/disk-final.json**.

Final allocated/apparent GiB: checkout **22.34/39.59**, `.reference`
**11.16/25.34**, `target` **9.71/12.53**; **71.72 GiB free**.
Every ceiling passes; six independent complete scans include hidden paths/.git
and every apparent hard link. Measurements are non-atomic during concurrent
work. No builds or parity captures follow the final audit.

### Lane I42: original physical Page glyph and source perimeter (2026-10-25)

Read the complete brief through **Update 33**, newest reports and STATUS;
followed superseding **31–32** scope. The remaining actionable image function,
**TestNativePhysicalPageReplacesFlatGlyphAndRetainsSourcePerimeter**, already
matches: **one original function / one call, before and final**, zero regressions.
Source: master **6a7b48378**, **native_page_test.go:50** and
**native_page.go:60–82**. No production change or golden gain.

The original Go assertions stay active. Replay uses the complete real dashed,
patterned Page source root, compares the entire resulting root, and checks
outer stroke identity preservation. Scene construction and Go allocation layout
are outside this observation. The ordinary native test and **three harness
tests pass**; the compressed executable repeats successfully. Its first test
setup used JSON Value equality, which distinguishes `1` from `1.0`; corrected
typed scene equality passes. External structural comparison matched throughout.
The initial failed log remains; no runtime improvement is attributed to it.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged**, standard delta **fixed 0 / regressed 0**. Frozen F79 CLI hashes
verify before/after. Other full tiers remain inherited; no full-parity claim.

Fresh inventory: **673 added / 577 named / 96 without** (576 named at start,
including concurrent Lane F observations). **Lane I 2 → 1 uncovered**:
**TestIconURLIdentityAndOwnership**. Its original source/unsupported witness
remains in **iso23-scene-qualified-capture**. `d2-rs-target::Url` retains JSON
`User: {}` but lacks owned username/password fields; Go restores them after
its JSON clone (**scene.go:182–200,329–343**). That target input boundary is
outside Lane I's edit scope. No exclusion or passing reclassification.

Evidence: **iso42/** `before.json`, `final.json`, `delta.json`, `scope.json`,
original source capture, native outputs/logs, **verify.py** and **retention.json**
(28 hashes including the unresolved URL witness). New alias:
**isometric-page-glyph-api.json**. Source restoration and input/expected/native
bindings verify. Shared test-runner initial/current hashes and rebuild are
explicit; no executable copy or source freeze. Capture limits:
**1 GiB total / 128 MiB file / 20 GiB free**, with finite deadlines.

Cleanup removed **four redundant files / 647,168 allocated bytes**; verified
Applesauce **0.5.30 / LZFSE** saves **1,437,696 bytes on six retained files**.
Bytes, modes, owners and original xattrs verify; clone backups removed.
Exact paths/replacements: **cleanup.json**, **compression.json**. Rollback:
`applesauce decompress --verify -- <exact path>`. Local APFS and idle exact-file
checks pass. No inherited corpus, shared cache, hard link or immutable object
changed; no bulk moved. Savings are operation/path footprints, separate from
concurrent net storage changes. No commits; other lanes and `29` preserved.

Final allocated/apparent GiB: checkout **22.34/39.59**, `.reference` **11.16/25.34**, `target` **9.71/12.53**; **71.72 GiB free**. All ceilings pass.
**iso42/disk-final.json** has six independent complete scans, including hidden
paths/.git and every apparent hard link. Measurements are non-atomic during
concurrent work. No builds or parity captures follow this audit.

### Lane I41: finish the interrupted route, opacity and background batch (2026-10-25)

Resumed the unreported I40 work after reading the complete brief through
**Update 33**, latest handoffs and STATUS; followed the **31–32** ownership split.
**20 original functions / 73 observations now match**, with zero regressions.
The I41 baseline was **18/20**. I40's initial **11/20** included three omitted
observer text-count inputs; those corrections are not native runtime gains.

The remaining queue/upright contact failures began at **geometry_routes.go:436**:
Go ARM64 contracts the compiled caption-width product plus its gap. Rust now
uses the same fused operation. Five isolated clipper calls already matched
when supplied identical inputs; wire construction matched too. The new ordinary
regression preserves five complete original Go contact-caption calls. I40's
source-mapped projection contraction is retained (**route_caption_mask.go:39**).
The resumed constructor adapter honors explicit density/options and keeps
vector-document retention separate from cap coverage (**geometry.go:431–480**).
Public SVG construction supplies the vector context; ordinary raster construction
does not. A native test covers the independent flags. The existing artwork test
bridge now supplies its original explicit vector context
(**vector_surface_test.go:340**); its initial failure is recorded and fixed.

| Gate | Final result | Regressions |
|---|---:|---:|
| Original functions / observations | **20/20 / 73/73** | **0** |
| Full native isometric SVG / source errors | **870 / 138** | **0** |
| All original PNG goldens | **20/20** | **0** |
| E2e SVG / expected errors | **933 / 11**, all **966 classifications** | **0** |

**51 native + 3 harness + 38 raster-harness tests pass**. No test exclusion or
lossy comparison was added; the five ordinary external replays stay ignored by
default and this batch executes explicitly. Other full tiers remain inherited;
no new golden gain or full API parity is claimed. Go cached allocation/pointer
layout and caller assertions outside observed calls remain unqualified.

Fresh inventory: **673 added / 564 named / 109 without** repository-wide.
**Lane I 22 → 2 uncovered**: physical Page glyph/perimeter removal and the
scene URL/Userinfo ownership handoff. Keep those visible; no reclassification.
No commits; other lanes and `29` preserved. Minimal shared artwork/raster changes
are test-only adapters for the constructor interface and residual observations.

Evidence: **iso41/** `qualified.json`, `replay-start.json`, `delta.json`,
`svg-qualified.json`, source hashes/runtime patch, compact ARM64 instructions,
`verification.json`, source/native test logs and `verify.py`. Original Go inputs
and expected outputs remain in **iso40/final-source2**; I40's initial negative
capture/native outputs remain. **515 bound paths**, complete pixel inventories,
source restoration and all e2e classifications verify after compression.
Alias: **isometric-residual-api.json**. Shared runner retirement/rebuild hashes
are explicit; no full source or executable freeze was added. Captures have
streaming **1 GiB total / 128 MiB per file / 20 GiB free** limits and deadlines.

Cleanup retires **3,481 files / 1961.82 MiB allocated path footprint**:
byte-verified successful SVG bodies (replaced by their pinned goldens and
reproduction inventories) and superseded full assembly (compact instructions and
fresh-path rebuild retained). **Applesauce 0.5.30 / LZFSE saves
166.63 MiB on 140 retained files**.
Hashes/modes/owners/original xattrs verify; clone backups were removed. Exact
paths/replacements and rollback: **cleanup.json**, **compression.json**;
`applesauce decompress --verify -- <path>`. Local APFS is verified; DiskManagement
is unavailable. No active file, hard link or immutable retention object was
rewritten; raw content-addressed input blobs were skipped. No bulk moved.
Savings are operation/path footprints, separate from concurrent net usage.

Final checkout **22.48/39.63**, `.reference` **11.20/25.38**, `target` **9.80/12.53 GiB allocated/apparent**; **70.95 GiB free**. Every ceiling passes
(**iso41/disk-final.json**, six independent complete scans, including hidden
paths/.git and every apparent hard link). Measurements are non-atomic during
concurrent lanes. No builds or parity captures follow the final audit.

### Lane I39: original ink, Markdown-card and opacity composition (2026-10-25)

Read the complete brief through **Update 33**, newest reports/messages and
STATUS, including arriving L40; followed the superseding Updates **31–32**.
**20 original functions / 146 observations match native Rust. No production
algorithm change was needed.** The batch covers authored/dashed/translucent
ink, branch joins and opposite-cap supports, all five remaining Markdown-card
construction/placement functions, and five opacity-composition functions.
Sources: master **6a7b48378**, **native_edges.go:102,429;
native_ink_joins.go:12; markdown_cards.go:12,26,33;
native_shapes.go:403; hierarchy_boards.go:77;
hierarchy_headers.go:25**, with raster.go.

| Gate | Result | Regressions |
|---|---:|---:|
| Original functions / observations | **20/20 / 146/146** | **0** |
| E2e SVG / expected errors | **933 / 11 unchanged** | **0** |
| Complete e2e classifications | **966 unchanged** | **0** |

**49 native tests, six new harness checks and 38 raster-harness checks pass**,
including every **20 PNG golden**. Library: **40 pass / four external replay
tests ordinarily ignored**; this batch's external replay runs explicitly.
Whole SVG **870/870** and other full tiers remain inherited. The frozen I25
CLI repeats every e2e classification; runtime code was unchanged. No golden
or full API parity gain is inferred from named observations.

Original Go test bodies/assertions remain active. Native ink compares complete
ordered geometry, normals/UVs, support facets and material/group/owner sharing.
Markdown calls also compare texture hashes, complete source nodes, face/rich
budgets, animation ranges, links and header placement. Opacity render/frame
calls consume original supplied Go meshes; native construction coverage and
Go cached-state/allocation assertions are separate. Complete nonempty raw
pixel stores, dimensions and byte hashes verify on both sides. Requests
contain inputs only; new bridges/helper visibility compile only in tests.
Two initial observer differences (bypassing the caption-tail ink wrapper and
nil-slice echo) were corrected; **they are not runtime fixes**. The first
raster-harness invocation lacked Pillow; the complete dependency-equipped
run passes all 38 checks.

Fresh inventory: **673 added / 541 named / 132 without**. **Lane I 42 → 22**
uncovered: **21 image functions plus one scene URL/Userinfo handoff**. No
exclusions added. Next prefer the remaining three opacity admission/shadow
functions and route-caption/contact contracts; put any real failure first.

Evidence: **iso39/** `final-source`, `before.json`, `final.json`, input-only
native requests, `delta.json`, `scope-final.json`, `source.json`, tests and
`verify.py`; alias **isometric-ink-card-api.json** preserves I13's older
composition alias. **53 original input/expected and 89 native artifact
bindings** verify, along with source restoration and all 966 e2e outcomes.
Captures use streaming **1 GiB total / 128 MiB file / 20 GiB free** limits
and finite deadlines. The shared Cargo runner is explicitly mutable;
`native-runner-retirement.json` records superseded hashes and rebuild command.
No executable/source-tree copies were added. Both initial negative observer
witnesses remain; retired successful draft paths have explicit replacements.

Cleanup deletes **81.52 MiB allocated / 143 files**, all this run's superseded
draft capture and duplicate successful replays. Verified **Applesauce
0.5.30 / LZFSE** saves **77.41 MiB / 87 retained files**, preserving SHA-256,
modes, owners and original xattrs; clone backups removed. Exact paths and
replacement/rollback records: **iso39/{cleanup,compression}.json**. Rollback:
`applesauce decompress --verify -- <path>`. Local APFS verified;
DiskManagement unavailable. No active/shared executable or cache rewritten,
hard link broken or bulk moved.

Final allocated/apparent GiB: checkout **21.93/39.17**, `.reference` **10.72/24.74**, `target` **9.74/12.71**; **72.10 GiB free**. Every ceiling passes (`iso39/disk-final.json`; six independent complete scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow the final audit.

### Lane I38: original SVG seams, visibility and ground receivers (2026-10-25)

Read the full brief through **Update 33**, newest reports/messages and STATUS;
followed the superseding Updates 31–32 split and larger batches. **20 original
functions / 1,053 input-only observations already match**; no production fix.
The batch covers six receiver/domain/snapshot/admission tests, all five seam
cover tests, three projection/link-rounding tests, two shadow-coverage tests,
shared surface resources, hidden faces, class-row visibility and Markdown
card exclusion. Every original Go assertion remains active. Source mappings:
`ground_receivers.go:9,71`, `svg_seams.go:19,57,120`, `svg.go:237,477`,
`svg_surface_resources.go:8`, `svg_visibility.go`, `raster.go`, `markdown_cards.go`.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / observations | **20/20 / 1,053/1,053 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

**49 native + six harness tests pass**, including all **20 PNG goldens** and
selected whole-board SVG/assets. Full library: **40 pass / three ordinary
external replays ignored**. The complete fixture passes again after verified
compression. All 966 e2e classifications repeat on the unchanged frozen I25
CLI. Full isometric SVG **870/870** and other full tiers remain inherited;
no new golden gain or full API parity claim. Additions compile only in tests,
including the minimal direct validator adapter in Lane F's `raster.rs`.

Complete ordered meshes, material/group/owner identities, receiver values,
texture pixel hashes, cover grouping, visibility fragments, exact errors and
full SVG bytes compare. Full retained vector documents feed the native SVG
writer. The 1,002 resource-definition calls exercise the original sharing
sequence. Go slice/nil-slice allocation and receiver-pointer identities are
unqualified because native planes/meshes are owned values. Caller shadow
raster assertions remain active in Go; this observer compares full SVG bytes
before their filter removal/rasterization. Those distinctions are explicit;
named observations do not prove all assertions or full API compatibility.
Initial call-site rewrite, concurrent gofmt/source-hash rejection and fixture
import-path setup are excluded from native results. No commits.

Fresh inventory: **673 added / 521 named / 152 without**, including two
new concurrent Lane F observations. **Lane I 62 → 42 total uncovered**
(41 image + one scene URL/Userinfo handoff); Lane L has two image functions.
Next prefer the remaining opacity-composition/ink contracts and five Markdown
card construction/placement functions; put actual failures first. No exclusions.

Evidence: `.reference/codex-runs/iso38/` `final-source`,
`before.json`, `qualified-final.json`, input-only `qualified-final-native`
requests, `delta.json`, tests and `verify.py`; baseline alias:
**isometric-coverage-api.json**. Original input/expected-output closure,
fixture membership, source restoration, all 966 classifications and five
compressed hashes/modes/owners verify. Streaming capture/replay limits:
**1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.
No executable body or full concurrent source freeze was added.

Cleanup removes **169 files / 59.26 MiB allocated path footprint** of failed
observer setup and redundant successful capture/replay outputs. Every
original expectation remains in `final-source`; replay requests remain input
only. Verified **Applesauce 0.5.30 / LZFSE** saves **27.95 MiB / five retained
files**; SHA-256, modes, owners and original xattrs verify, clone backups are
removed. Exact targets, deletion replacements and mutable-runner bindings:
`iso38/{cleanup,compression,runner-retirement}.json`; rollback:
`applesauce decompress --verify -- <path>`. Local APFS verified; DiskManagement
unavailable. No shared cache/executable removed or compressed, hard links
broken or bulk moved. Savings are operation/path footprints during concurrent work.

Final allocated/apparent GiB: checkout **21.88/39.02**, `.reference` **10.70/24.64**, `target` **9.71/12.66**; **82.78 GiB free**. Every ceiling passes (`iso38/disk-final.json`; six independent complete scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L68: actionable assignment required (2026-10-25)

Re-read the full brief through **Update 33**, including its initial Update,
newest L67/F85/I51 reports/messages and arriving I52. **Updates 31–32
supersede Update 26.** Fresh inventory remains **673 added / 588 named /
85 without; Lane L 109/111 observed**. Six source/type hashes and the L39
residual hash are unchanged. The remaining original functions are
**TestNativeRasterAdmissionAndCancellation** (three typed-nil Go image
interfaces) and **TestNativeGIFLoopCloneAdmission** (two cyclic scene inputs).
Native `Option<Rc<Texture>>` and owned `Vec<Node>` cannot represent those
inputs; both functions remain visible and non-passing. No new API captures,
runtime changes, observations, builds or commits. An actionable replacement
assignment was requested asynchronously. **Do not launch another unchanged
Lane L confirmation run; assign a failing test/package first.**

Required retained-F79 e2e: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, **fixed 0 / regressed 0** against L53 and the
current shared baseline. This verifies the hash-bound executable, not
concurrent source. Other tiers inherited. Streaming **1 GiB total / 128 MiB
file / 20 GiB free** guards and finite deadlines apply. Evidence:
**lang68/**; handoff: **report-lang68.md**. Cleanup removes only this run's
duplicate report/raw inventory: **475,136 allocated bytes / 0.453 MiB**;
paths, hashes and replacement evidence in **lang68/cleanup.json**. No shared
cache/corpus retirement, compression writes/backups or bulk relocation.
Final allocated/apparent GiB: checkout **22.62/39.88**, `.reference`
**11.37/25.55**, `target/` **9.77/12.60**; **70.57 GiB free**. All ceilings
pass (**lang68/disk-final.json**; six independent complete scans include
hidden paths/.git and every apparent hard link, non-atomic during concurrent
work). `check.py verify` passes. No builds or parity captures follow this audit.

### Lane L67: exhausted scope handoff (2026-10-25)

Re-read the complete brief through **Update 33**, including its initial
Update, newest L66/I50/F85 reports/messages and current status. Updates
**31–32 supersede Update 26**. Fresh inventory is unchanged: **673 added /
588 named / 85 without**; Lane L **109/111 observed**. Six source/type hashes
and the original L39 residual hash verify unchanged. The remaining original
functions are `TestNativeRasterAdmissionAndCancellation` (three Go typed-nil
image interfaces) and `TestNativeGIFLoopCloneAdmission` (two cyclic-scene
inputs); current native types cannot represent those inputs. Both remain
uncovered. No runtime edits, new observations, builds or commits.

Required retained-F79 executable guard: **933 SVG + 11 expected errors**,
all **966 classifications unchanged**, **fixed 0 / regressed 0** against L53
and the current shared baseline. This verifies that executable, not concurrent
source changes. Evidence: **lang67/**; report: **report-lang67.md**.
Deleted this run's duplicate report/raw inventory after verification:
**475,136 allocated bytes / 0.453 MiB**, with paths/hashes/replacements in
`lang67/cleanup.json`. No shared cache/corpus retirement or bulk relocation.
New assignment requested; no response received. Further unchanged-scope
confirmation runs cannot advance implementation and should await reassignment.

Final allocated/apparent GiB: checkout **22.53/39.79**, `.reference` **11.34/25.52**, `target/` **9.71/12.55**; **69.43 GiB free**. All ceilings pass.
Exact measurements: **lang67/disk-final.json** (six independent complete
scans; hidden paths/.git and all apparent hard links included; non-atomic
during concurrent work). No builds or parity captures follow this audit.

### Lane L66: assignment exhausted; no runtime changes (2026-10-25)

Read the full brief through **Update 33**, initial Update included, newest
L65/I49/F84 reports/messages and arriving I50. **Updates 31–32 supersede
Update 26**. Fresh inventory: **673 added / 588 named / 85 without**;
Lane L remains **109/111**. The eight new repository observations are
concurrent ELK-option/CLI-mode work, not L66 progress. Six source/type hashes
and the L39 residual hash remain unchanged. The two residual functions still
require three Go typed-nil image interfaces and two cyclic-scene inputs
unavailable in current Rust types. Neither is excluded or counted passing.
No production edits, builds, API recaptures, new observations or commits.
New assignment requested; none received. Further unchanged-scope runs should
stop until reassignment.

Required frozen-F79 e2e: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, **fixed 0 / regressed 0** against retained L53
and the current shared baseline. This verifies that executable, not concurrent
source changes. Evidence: **lang66/**; handoff **report-lang66.md**.
Deleted only this run's duplicate report/raw inventory, **0.453 MiB allocated**;
exact paths/hashes and replacements in cleanup.json. No shared/corpus retirement
or bulk relocation. Read-only Applesauce inspection confirms retained L53
already uses LZFSE; no compression writes/backups.

Final allocated/apparent GiB: checkout **22.53/39.79**, `.reference` **11.34/25.52**, `target/` **9.71/12.55**; **69.81 GiB free**. All ceilings pass (**lang66/disk-final.json**; six independent full scans include hidden paths/.git and every apparent hard link, non-atomic during concurrent work). No builds or parity captures follow this audit.

### Lane L65: no remaining actionable assignment (2026-10-25)

Read the full brief through **Update 33**, initial Update included, and latest
L64/I48/F84 reports/messages. **Updates 31–32 supersede Update 26**. Fresh
inventory remains **673 added / 580 named / 93 without**, Lane L **109/111**.
Six source/native type hashes and the L39 residual witness hash are unchanged.
The two residual functions still require three Go typed-nil image interfaces
and two cyclic-scene inputs unavailable in current Rust types. Neither is
excluded or counted passing. No runtime edits, builds, API recaptures, new
observations or commits. New assignment requested; none received. Further
unchanged-scope confirmation runs should stop until reassignment.

Required frozen-F79 e2e guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, **fixed 0 / regressed 0** against retained L53
and the shared baseline. This verifies that executable, not concurrent source.
Evidence: **lang65/**; report **.reference/codex-runs/report-lang65.md**.
Final allocated/apparent GiB: checkout **22.59/39.79**, `.reference` **11.35/25.52**, `target/` **9.76/12.55**; **68.28 GiB free**. All ceilings pass (**lang65/disk-final.json**; six independent full scans include hidden paths/.git and all apparent hard links, non-atomic during concurrent work). Cleanup recovered **0.449 MiB allocated** in deleted-path footprints, separate from concurrent net changes. No builds or parity captures follow this audit.

### Lane L64: current scope exhausted; handoff (2026-10-25)

Re-read the full brief through **Update 33**, latest reports/messages including
arriving F84/I48, and current status. **Updates 31–32 supersede Update 26**.
Fresh inventory: **673 added / 580 named / 93 without**; Lane L remains
**109/111 observed**, with no new coverage or runtime edits. Six pinned
source/native type hashes and the L39 residual witness hash remain unchanged.
The remaining functions require three Go typed-nil image inputs and two
cyclic-scene inputs unavailable in current Rust types; neither is counted
passing. New assignment requested; no scope extension received. Do not repeat
unchanged-scope confirmation runs as implementation progress.

Required frozen-F79 e2e guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, **fixed 0 / regressed 0** against retained L53
and the current shared baseline. This checks that executable, not concurrent
source changes. No builds, API recaptures or commits. Evidence/verification:
**lang64/check.py**, source-validation/scope/delta/cleanup/disk JSON and logs;
handoff **.reference/codex-runs/report-lang64.md**. Deleted only two own
duplicate outputs, **0.449 MiB allocated**; retained L53 baseline already uses
LZFSE (Applesauce 0.5.30, read-only inspection). No corpus/cache retirement.

Final allocated/apparent GiB: checkout **22.49/39.75**, `.reference`
**11.31/25.48**, `target` **9.71/12.55**; **67.52 GiB free**. All ceilings
pass (**lang64/disk-final.json**, six independent scans including hidden
paths/.git and every apparent hard link; non-atomic during concurrent work).

### Lane L63: assignment exhausted; regression guard unchanged (2026-10-25)

Read the complete brief through **Update 33**, latest L62/I46/F83 handoffs,
arriving I47, and current STATUS/PLAN/workflow. **Updates 31–32 supersede
Update 26**. Fresh inventory remains **673 added / 579 named / 94 without**;
Lane L remains **109/111 observed**, with no new observations or runtime edits.
Ordered by unsupported call count, the residuals are three typed-nil image
calls in **TestNativeRasterAdmissionAndCancellation** and two cyclic-scene
calls in **TestNativeGIFLoopCloneAdmission**. Six pinned source/native type
hashes still match L62; original witnesses remain **lang39/residuals.json**.
Neither function is excluded or counted passing. No actionable failing
cluster remains in the assigned scope; newer scope requested asynchronously.
No builds, repeated API captures or commits. Do not repeat unchanged-scope
confirmation runs as implementation work.

Mandatory retained-F79 guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, standard delta **fixed 0 / regressed 0** against
L53 and the current shared baseline. Runner hash verifies before/after;
this qualifies the retained executable, not concurrent source changes.
Other full tiers are inherited. Streaming limits: **1 GiB total / 128 MiB
file / 20 GiB free**. Evidence and verification: **lang63/check.py**,
**source-validation.json**, scope/delta/run/cleanup JSON and logs; handoff:
**.reference/codex-runs/report-lang63.md**. Only this section is a tracked
edit; other lanes and stray `29` are preserved. Deleted two own duplicate
outputs, **471,040 allocated bytes / 0.449 MiB**, after replacement checks;
retained L53 baseline already uses **LZFSE** (Applesauce **0.5.30**, read-only
inspection). No shared cache/corpus retirement or compression rewrite.

Final allocated/apparent GiB: checkout **22.56/39.81**, `.reference`
**11.31/25.48**, `target` **9.78/12.60**; **68.28 GiB free**. Every ceiling
passes (**lang63/disk-final.json**, six independent complete scans including
hidden paths/.git and all apparent hard links; non-atomic during concurrent
work). Cleanup recovered **0.449 MiB allocated**. No builds or parity captures
follow this audit.

### Lane L62: scope handoff, no implementation change (2026-10-25)

Re-read the full brief through **Update 33**, latest L61/F83/I45 handoffs
and current status. Updates **31–32 supersede 26**; no new assigned cluster
exists. Fresh inventory: **673 added / 579 named / 94 without**; Lane L
remains **109/111 observed**. The same typed-nil raster images (three calls)
and cyclic GIF scene (two calls) remain uncovered; pinned source/native type
hashes match L61. Neither residual is excluded or counted passing. No runtime
edits, builds, repeated API captures, new observations or commits.

Mandatory frozen-F79 e2e guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, standard delta **fixed 0 / regressed 0** against
L53 and the current shared baseline. This checks the retained executable,
not concurrent source changes; other full tiers are inherited. New scope
requested asynchronously; no answer assumed. Do not schedule another
unchanged-scope confirmation run as implementation work. Report, actual
deletion ledger and final disk audit: **.reference/codex-runs/report-lang62.md**
and **lang62/**. Only this section is a tracked edit; other lanes preserved.

Final allocated/apparent GiB: checkout **22.49/39.74**, `.reference` **11.30/25.47**, `target` **9.72/12.55**; **68.28 GiB free**. Every ceiling passes (**lang62/disk-final.json**; six independent complete scans include hidden paths/.git and every apparent hard link; non-atomic during concurrent work). Cleanup recovered **0.449 MiB allocated**. No builds or parity captures follow this audit.

### Lane L61: latest assignment remains exhausted (2026-10-25)

Read the complete brief through **Update 33**, newest L60/I45/F82 handoffs
and current STATUS. Updates **31–32 supersede 26**. Fresh inventory remains
**673 added / 579 named / 94 without**; Lane L remains **109/111 observed**.
By residual call count, raster admission has three typed-nil Go image inputs;
GIF clone admission has two cyclic-scene calls. Pinned source/native type
hashes still match L59; neither residual is representable by current owned
Rust types, excluded or counted passing. No actionable failing cluster remains
in this assignment. No runtime edits, builds, new API captures, observations
or commits; this is a scope handoff, not implementation progress.

Required frozen-F79 e2e guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**, standard delta **fixed 0 / regressed 0** against
both L53 and the latest F83 scoreboard. This checks the retained executable,
not a rebuild of concurrent changes. Other full tiers inherited. Evidence,
cleanup ledger and final storage audit: **.reference/codex-runs/report-lang61.md**
and **lang61/**. Only this STATUS section is a tracked edit; other lanes and
`29` preserved. A newer assignment was requested; no scope extension assumed.

Final allocated/apparent GiB: checkout **22.48/39.74**, `.reference` **11.30/25.47**, `target` **9.71/12.55**; **66.38 GiB free**. Every ceiling passes (**lang61/disk-final.json**; six independent complete scans include hidden paths/.git and every apparent hard link; non-atomic during concurrent work). No builds or parity captures follow this audit.

### Lane L60: scope handoff; no new failing cluster (2026-10-25)

Re-read the full brief through Update 33 and latest L59/I45/F82 reports.
Updates **31–32 supersede 26**. Fresh inventory: **673 added / 579 named /
94 without**. Lane L remains **109/111 observed**; its typed-nil raster
images and cyclic GIF scene inputs remain uncovered and unrepresentable in
the current Rust types. Existing witnesses and source/type hashes verify.
No runtime changes, new observations, repeated API captures, builds or commits.
This is a scope handoff; further implementation needs a new assigned cluster.
Full evidence: **.reference/codex-runs/report-lang60.md**.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged**; standard delta **fixed 0 / regressed 0**, unchanged frozen F79
runner hash. Other full tiers inherited. Removed this run's duplicate
report/inventory only: **471,040 allocated bytes / 0.449 MiB**, recorded with
replacement paths and hashes in **lang60/cleanup.json**. Read-only Applesauce
inspection confirms the retained report is already LZFSE compressed; no rewrite.

Final allocated/apparent GiB: checkout **22.52/39.73**, `.reference` **11.29/25.47**, `target` **9.76/12.55**; **71.33 GiB free**. Every ceiling passes (**lang60/disk-final.json**; independent complete scans include hidden paths/.git and every apparent hard link, non-atomic during concurrent work). No builds or parity captures follow this audit.

### Lane L59: assignment still exhausted (2026-10-25)

Read the complete brief through Update 33 and newest L58/I44/F82 reports.
Updates **31–32 supersede 26**. Fresh inventory **673/577/96**; Lane L remains
**109/111 observed**, with typed-nil raster images and cyclic GIF scene inputs
uncovered. Original witnesses and unchanged source/type hashes verify; no
actionable failing cluster, runtime change, new observation, build or API capture.
Requested scope guidance asynchronously; no response assumed. Full details and
fresh regression/storage evidence: **.reference/codex-runs/report-lang59.md**.
This is a scope handoff, with no implementation progress claimed.
E2e **933 SVG + 11 expected errors**, **966 unchanged classifications**;
standard delta **fixed 0 / regressed 0**. Retired this run's duplicate
report/inventory (**0.449 MiB allocated**); retained report/runner hashes verify.

Final allocated/apparent GiB: checkout **22.31/39.56**, `.reference` **11.18/25.35**, `target` **9.66/12.49**; **71.72 GiB free**. Every ceiling passes (**lang59/disk-final.json**). Six independent complete scans include hidden paths/.git and every apparent hard link; non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L58: current assignment exhausted; refreshed guard (2026-10-25)

Re-read the full brief through Update 33, newest reports/messages and the
current status. Updates **31–32 supersede Update 26**. Fresh inventory remains
**673 added / 577 named / 96 without**; Lane L remains **109/111 observed**.
The same two functions remain uncovered: **TestNativeRasterAdmissionAndCancellation**
(three typed-nil Go image inputs) and **TestNativeGIFLoopCloneAdmission**
(a self-cyclic scene). Source comparison confirms that `Option<Rc<Texture>>`
and owned `Vec<Node>` cannot express those original inputs. Existing witnesses:
**lang39/residuals.json**; refreshed pinned source/native hashes: **lang58/source.json**.
Neither function is excluded or counted passing. New assignment/API scope
requested asynchronously; no response assumed. No runtime edits, builds,
new API captures, observations or commits. No implementation progress claimed.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged**, standard delta **fixed 0 / regressed 0**. Frozen F79 CLI hash
verified before/after against L57. Other full tiers remain inherited. Evidence:
**.reference/codex-runs/report-lang58.md**, **lang58/{scope,source,delta,e2e-run,cleanup}.json**,
logs and **check.py verify**. Deleted this run's redundant report/inventory:
**471,040 allocated bytes / 0.449 MiB**, with exact hashes/replacements in
cleanup.json. Retained identical L53 report is already LZFSE compressed
(Applesauce 0.5.30 inspection). No shared caches/corpora changed or bulk moved.
Fresh storage audit: **lang58/disk-final.json**; final numbers in the report.
Other lanes and `29` preserved; tracked edit is this STATUS section only.

Final allocated/apparent GiB: checkout **22.32/39.57**, `.reference` **11.17/25.35**, `target` **9.68/12.50**; **71.72 GiB free**. Every ceiling passes. Six independent complete scans include hidden paths/.git and all apparent hard links; measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L57: unchanged scope exhausted; handoff (2026-10-25)

Read the full brief, including its initial Update and Updates through **33**,
latest L56/I43/F82 reports/messages and current STATUS/PLAN/workflow.
**Updates 31–32 supersede Update 26.** Fresh inventory remains **673 added /
577 named / 96 without**; Lane L remains **109/111 named, two uncovered**.
Confirmed the original typed-nil-image and self-cyclic-scene inputs against
pinned source and current owned Rust types. **lang39/residuals.json** retains
both witnesses/source mappings; neither is excluded or counted passing.
Requested new scope asynchronously; no reply is assumed. No actionable failing
cluster remains in the assignment. No implementation progress is claimed.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged**; standard delta **fixed 0 / regressed 0**. Frozen F79 CLI SHA-256
matches L56 and verifies before/after. No runtime edits, builds, new Go/API
captures, native observations, exclusions or commits. Other full tiers inherited;
other lanes and `29` preserved. Supervisor tracked path: **this L57 section only**.

Evidence: **.reference/codex-runs/report-lang57.md**, **lang57/{scope,delta,
e2e-run,cleanup}.json**, **check.py**, e2e/delta logs. Fresh report was
byte-identical to retained, already-compressed L53 evidence; replacement and
runner hashes verify after cleanup. Streaming **1 GiB total / 128 MiB file /
20 GiB free** guards, finite deadlines. Deleted only this run's duplicate
inventory/report: **471,040 allocated bytes / 0.449 MiB**; exact hashes,
replacements and rebuild commands in cleanup.json. No shared cache/corpus,
immutable object or executable retired; no bulk moved or compression backup.

Final allocated/apparent GiB: checkout **22.34/39.59**, `.reference`
**11.16/25.34**, `target` **9.71/12.53**; **71.72 GiB free**.
Every ceiling passes (**lang57/disk-final.json**, six independent complete
scans including hidden paths/.git and every apparent hard link; non-atomic
with concurrent work). No builds/captures follow the final audit.

### Lane L56: exhausted scope; reassignment still needed (2026-10-25)

Read the complete brief through **Update 33**, including its initial Update,
latest L55/I42/F81 reports/messages and current status. **Updates 31–32
supersede Update 26.** Fresh inventory: **673 added / 577 named / 96 without**;
Lane L remains **109/111 named, two uncovered**. The additional named
observation belongs to Lane I. Original typed-nil Go image interfaces and
self-cyclic scene inputs remain unrepresentable by current owned Rust types;
neither residual is excluded or counted passing. Original witnesses/source
mapping remain in **lang39/residuals.json**. Requested new scope; the current
assignment has no actionable failing cluster. Do not schedule another
unchanged-scope confirmation run as implementation work.

Required session e2e guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged**; standard delta **fixed 0 / regressed 0**.
Unchanged frozen F79 CLI hash verifies before/after and matches L55.
No runtime edits, builds, new Go/API captures, native observations, exclusions
or commits. Other full tiers are inherited; other lanes and `29` preserved.
Supervisor tracked path: **this L56 section only**.

Evidence: **.reference/codex-runs/report-lang56.md**, **lang56/{scope,delta,
e2e-run,cleanup}.json**, **check.py** and e2e/delta logs. Fresh report bytes
match retained, already-compressed L53 evidence; replacement/runner hashes
verify after cleanup. Streaming **1 GiB total / 128 MiB file / 20 GiB free**
limits and finite deadlines. Deleted only this run's redundant inventory/report:
**471,040 allocated bytes / 0.449 MiB**, with exact hashes/replacements/rebuild
commands in cleanup.json. No shared artifact or corpus closure changed;
no new bulk or compression backups. Process inspection remains unavailable,
so no shared-cache cleanup was attempted. Final allocated/apparent GiB:
checkout **22.34/39.59**, `.reference` **11.16/25.34**, `target` **9.71/12.53**;
**71.72 GiB free**. Every ceiling passes (**lang56/disk-final.json**, six
independent complete scans including hidden paths/.git and every apparent
hard link). Measurements are non-atomic during concurrent work.
No builds/captures follow that audit.

### Lane L55: exhausted assignment; new scope requested (2026-10-25)

Read the complete brief through **Update 33**, including its initial Update,
latest L54/I41/F81 handoffs and current status. **Updates 31–32 supersede
Update 26**. Fresh inventory: **673 added / 576 named / 97 without**; Lane L
remains **109/111 named, two uncovered**. Repository-wide gains belong to
other lanes. The original typed-nil-image and self-cyclic-scene inputs remain
uncovered; retained witnesses/source mapping are in **lang39/residuals.json**.
No actionable failing cluster remains under this assignment. Requested new
scope; do not repeat completed API captures or treat unchanged-scope runs as
implementation progress.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged**; standard delta **fixed 0 / regressed 0**. Frozen F79 CLI hash
matches L54 and verifies before/after. No runtime changes, builds, new Go/API
captures, native observations, exclusions or commits. Other full tiers are
inherited. Supervisor tracked path: **this L55 section only**.

Evidence: **.reference/codex-runs/report-lang55.md**, **lang55/{scope,delta,
e2e-run,cleanup}.json**, **check.py**, e2e/delta logs. Byte-identical fresh
report binds to retained, already-compressed L53 evidence; runner and retained
report hashes verify after cleanup. Capture limits: streaming **1 GiB total /
128 MiB file / 20 GiB free**, finite deadlines. Deleted only this run's
redundant inventory/report: **471,040 allocated bytes / 0.449 MiB**. Exact
hashes/replacements/rebuild commands are in cleanup.json. No shared artifacts,
corpus closure or immutable objects changed; other lanes and `29` preserved.
Process inspection is unavailable (`ps: operation not permitted`), so no
shared-cache cleanup attempted. Final allocated/apparent GiB: checkout
**22.34/39.59**, `.reference` **11.16/25.34**, `target` **9.71/12.53**;
**71.72 GiB free**. Every ceiling passes (**lang55/disk-final.json**; six
independent complete scans including hidden directories/.git and every
apparent hard link). Measurements are non-atomic during concurrent work.
No builds/captures follow the final audit.

### Lane L54: unchanged assignment exhausted; handoff (2026-10-25)

Re-read the complete brief, including the initial Update and Updates through
**33**, latest L53/I41/F81 reports/messages and current status. **Updates
31–32 supersede Update 26**. Fresh inventory remains **673 added / 564 named /
109 without**; Lane L is **109/111 named, two uncovered**. Confirmed the
original typed-nil-image and self-cyclic-scene inputs against pinned source
and current Rust types; L39 retains the witnesses/source mapping. These two
functions remain uncovered. No actionable failing board remains in the
assignment; requested new scope. Do not repeat completed API captures or
start another unchanged-scope confirmation run as implementation progress.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged**; delta **fixed 0 / regressed 0**. Frozen F79 CLI hash matches L53
and verifies before/after. No runtime edits, builds, new Go captures, native
observations, exclusions or commits. Other full tiers remain inherited.
Supervisor tracked path: **docs/STATUS.md, L54 section only**.

Evidence: **.reference/codex-runs/report-lang54.md**, `lang54/{scope,delta,
e2e-run,cleanup}.json`, `check.py`, e2e log and hash-verified report binding.
Limits: streaming **1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.
Deleted this run's redundant raw inventory and byte-identical report:
**471,040 allocated bytes (0.449 MiB)**. Report bytes bind to retained,
already-LZFSE-compressed L53 evidence; no new bulk retained or compression
backup created. No shared artifact, corpus binding or immutable object
changed; other lanes and `29` preserved. Savings are path footprints,
separate from concurrent checkout changes. Final allocated/apparent GiB:
checkout **22.30/39.45**, `.reference` **10.96/25.14**, `target` **9.86/12.58**;
**71.33 GiB free**. Every ceiling passes (**lang54/disk-final.json**, six
independent complete scans including hidden paths/.git and every apparent
hard link). Measurements are non-atomic during concurrent work. No builds
or parity captures follow this audit.

### Lane L53: exhausted scope; unchanged regression guard (2026-10-25)

Read the full brief through **Update 33**; **31–32 supersede Update 26**.
Fresh inventory: **673 added / 564 named / 109 without**. Lane L remains
**109/111 named; two uncovered**, unchanged from L52. Original typed-nil-image
and self-cyclic-scene inputs remain unrepresentable by current Rust owned
types; L39 retains their witnesses and source mapping. No actionable failing
board remains. Reassignment requested; no runtime changes, builds, new native
observations, Go captures, exclusions or commits.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged** from L52; standard delta **fixed 0 / regressed 0**. Frozen F79
CLI hash matches L52 and verifies before/after. Other full tiers inherited.
Evidence: **.reference/codex-runs/report-lang53.md**, `lang53/{scope,delta,
e2e-run,cleanup,compression}.json`, report/log and check.py. Streaming limits:
**1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.

Deleted redundant inventory (**307,200 allocated bytes**); verified Applesauce
LZFSE saves **147,456 bytes**, preserving SHA-256/mode/owner/original xattrs.
Clone backup removed; total **0.434 MiB** path savings. Shared artifacts and
other lanes' edits preserved. No corpus binding retired or bulk moved.

Final allocated/apparent GiB: checkout **22.54/39.69**, `.reference` **11.20/25.38**, `target` **9.86/12.58**; **70.95 GiB free**. Every ceiling passes (`lang53/disk-final.json`; six independent complete scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L52: current assignment exhausted; regression guard (2026-10-25)

Read the full brief through **Update 33**; Updates **31–32** supersede
Update 26. Fresh inventory: **673 added / 564 named / 109 without**.
Lane L remains **109/111 named; two uncovered**. L39 retains the typed-nil
image/self-cyclic-scene witnesses, unrepresentable by current Rust types.
No actionable failing board remains; new scope requested. No runtime edits,
new observations, builds, Go captures, exclusions or commits.

E2e: **933 SVG + 11 expected errors**, all **966 classifications unchanged**
from L51; standard delta **fixed 0 / regressed 0**. Frozen F79 hash verifies.
Other full tiers inherited. Evidence: **.reference/codex-runs/report-lang52.md**,
`lang52/{scope,delta,e2e-run,cleanup,compression}.json` and report/log.
Capture limits: **1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.
Removed redundant inventory/draft (**311,296 allocated bytes**); verified
Applesauce LZFSE saves **147,456 bytes**, preserving bytes/mode/owner/xattrs.
Backup removed; total **0.438 MiB** path savings. Shared artifacts preserved.

Final allocated/apparent GiB: checkout **22.62/39.74**, `.reference` **11.35/25.50**, `target` **9.80/12.52**; **70.95 GiB free**. Every ceiling passes (`lang52/disk-final.json`; six independent complete scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L51: latest scope exhausted; zero-regression guard (2026-10-25)

Re-read the complete brief through **Update 33** and latest reports/messages.
**Updates 31–32 supersede Update 26**. Fresh inventory: **673 added / 544
named / 129 without**; Lane L remains **109/111 named, two uncovered**.
L39 retains the typed-nil-image and self-cyclic-scene witnesses and source
mapping; their original inputs remain unrepresentable by current Rust types.
No actionable failing board remains in this assignment; reassignment requested.
No runtime edits, new observations, builds, Go captures, exclusions or commits.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged** from L50; standard delta **fixed 0 / regressed 0**. Frozen F79 CLI
hash verifies before/after. Other tiers remain inherited. Evidence and rerun
commands: **.reference/codex-runs/report-lang51.md**, `lang51/{scope,delta,
e2e-run,cleanup,compression}.json`, report/log and check.py. Streaming limits:
**1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.

Removed redundant inventory (**303,104 allocated bytes**) and empty owned
scratch. Verified Applesauce **0.5.30 / LZFSE** saves **147,456 bytes** on the
completed e2e report; SHA-256, mode, owner and original xattrs preserved,
clone backup removed. Total **0.430 MiB** operation/path savings. No shared
artifact, hard link or retained corpus binding changed; no bulk moved.

Final allocated/apparent GiB: checkout **22.40/39.57**, `.reference` **11.20/25.34**, `target` **9.73/12.50**; **72.48 GiB free**. Every ceiling passes (`lang51/disk-final.json`; six independent complete scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L50: assignment exhausted; regression guard unchanged (2026-10-25)

Read the full brief through **Update 33**, including the initial Update,
newest reports/messages and STATUS. **Updates 31–32 supersede Update 26**.
Fresh inventory remains **673 added / 544 named / 129 without**; Lane L
remains **109/111 named, two uncovered**. The original typed-nil image and
self-cyclic scene inputs remain unrepresentable in the current owned Rust
types; L39 retains their witnesses and source mappings. No repeated
investigation, new observations, runtime edits, builds, Go captures,
exclusions or commits. A new Lane L assignment is needed for implementation.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged** from L49; standard delta **fixed 0 / regressed 0**. Frozen F79 CLI
SHA-256 verifies before/after. Other tiers remain inherited. Evidence:
**.reference/codex-runs/report-lang50.md**, `lang50/{scope,delta,e2e-run}.json`,
report/log, check.py and disk audits. Capture limits: streaming **1 GiB total /
128 MiB file / 20 GiB free**, finite deadlines. Only this section is tracked.

Removed redundant raw inventory (**303,104 allocated bytes**) and empty owned
scratch; `lang50/cleanup.json` records the replacement/rebuild. Verified
Applesauce **0.5.30 / LZFSE** saves **147,456 bytes** on the completed report,
preserving SHA-256, mode, owner and original xattrs; clone backup removed.
Exact target/rollback: `lang50/compression.json`. Total **0.430 MiB** operation
savings; no shared artifacts, hard links or corpus bindings changed.
Final allocated/apparent GiB: checkout **22.46/39.51**, `.reference` **11.30/25.31**, `target` **9.69/12.47**; **71.72 GiB free**. Every ceiling passes (`lang50/disk-final.json`). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L49: unchanged scope; reassignment requested (2026-10-25)

Read the complete brief through **Update 33**, including the initial Update,
latest reports/messages and STATUS. **Updates 31–32 supersede Update 26**.
Fresh inventory: **673 added / 544 named / 129 without**; Lane L remains
**109/111 named, two uncovered**. L39's typed-nil image and self-cyclic scene
witnesses remain unrepresentable by current owned Rust types; no repeated
investigation, substitute inputs or new native coverage. No runtime changes,
builds, Go captures, exclusions or commits. Reassignment requested; do not
schedule further implementation runs under this unchanged assignment.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged** from L48; standard delta **fixed 0 / regressed 0**. Frozen F79 CLI
hash verifies before/after. Other tiers inherited. Evidence and reproduction:
**.reference/codex-runs/report-lang49.md**, `lang49/{scope,delta,e2e-run}.json`,
e2e report/log and check.py. Streaming **1 GiB total / 128 MiB file / 20 GiB
free** guards and finite deadlines. Only this STATUS section is a tracked edit.

Deletion ledger `lang49/cleanup.json`: redundant raw inventory removed
(**303,104 allocated bytes**) plus empty owned scratch. Applesauce **0.5.30 /
verified LZFSE** saves **147,456 bytes** on the completed e2e report, preserving
hash/mode/owner/original xattrs; clone backup removed. Exact target/rollback
in `lang49/compression.json`. Total **0.430 MiB** path/operation savings,
separate from concurrent checkout changes. No shared artifacts, hard links
or corpus bindings rewritten or retired; no bulk moved.

Final allocated/apparent GiB: checkout **23.40/40.47**, `.reference` **12.27/26.28**, `target` **9.66/12.47**; **71.33 GiB free**. All ceilings pass (`lang49/disk-final.json`; six independent scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L48: current scope needs reassignment (2026-10-25)

Read the full brief, including the initial Update and Updates through **33**,
latest L47/L46/F81/I39 reports/messages and STATUS. **Updates 31–32 supersede
Update 26.** Fresh inventory remains **673 added / 544 named / 129 without**;
Lane L remains **109/111 named, two uncovered**. No runtime changes, new
observations, exclusions, builds, Go captures or commits.

The remaining functions are **TestNativeRasterAdmissionAndCancellation**
(three typed-nil Go image pointers) and **TestNativeGIFLoopCloneAdmission**
(a self-cyclic scene). Their original inputs have no representation in the
current owned Rust types. L39 preserves witnesses and source mappings; no
repeat investigation or substitute inputs. Scope reassignment requested;
there is no actionable failing board in this assignment.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged**, standard delta **fixed 0 / regressed 0** against L47. Frozen F79
CLI hash verifies before/after. Other tiers remain inherited. Evidence:
**.reference/codex-runs/report-lang48.md**, `lang48/{scope,delta,e2e-run}.json`,
e2e report/log, check.py and disk audits. Capture limits: **1 GiB total /
128 MiB file / 20 GiB free**, finite deadlines.

Deleted the redundant raw inventory (**303,104 allocated bytes**) and empty
owned scratch directory; replacement/rebuild in `lang48/cleanup.json`.
Verified Applesauce **0.5.30 / LZFSE** saves **147,456 bytes** on the completed
e2e report; SHA-256, mode, owner and original xattrs preserved, clone backup
removed (`lang48/compression.json`). Rollback: `applesauce decompress --verify
-- .reference/codex-runs/lang48/e2e/report.json`. Local APFS verified;
DiskManagement unavailable. Total **0.430 MiB** path/operation savings, separate
from concurrent checkout changes. No shared artifact, hard link or corpus
binding rewritten or retired; other lanes and `29` preserved; no bulk moved.

Final allocated/apparent GiB: checkout **22.67/39.90**, `.reference` **11.33/25.34**, `target` **9.87/12.84**; **71.33 GiB free**. All ceilings pass (`lang48/disk-final.json`; six independent scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L47: unchanged assignment; reassignment requested (2026-10-25)

Read the full brief, initial Update and Updates through **33**, latest
L46/L45/F81/I39 reports/messages and STATUS. **Updates 31–32 supersede Update
26.** Fresh inventory remains **673 added / 544 named / 129 without**;
Lane L remains **109/111 named, two uncovered**. No runtime changes, new
observations, exclusions, builds, Go captures or commits.

The residuals are **TestNativeRasterAdmissionAndCancellation** (three typed-nil
Go images) and **TestNativeGIFLoopCloneAdmission** (self-cyclic scene).
L39 retains the original witnesses/source mappings; current owned Rust types
cannot express those inputs. No repeated investigation or substitute inputs.
Requested a new Lane L scope; none arrived during this run. Do not schedule
another implementation run under this unchanged assignment.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged**, standard delta **fixed 0 / regressed 0** against L46. Frozen F79
CLI hash verifies before/after. Other tiers remain inherited. Evidence:
**.reference/codex-runs/report-lang47.md**, `lang47/{scope,delta,e2e-run}.json`,
e2e report/log, check.py and disk audits. Streaming limits: **1 GiB total /
128 MiB file / 20 GiB free**, finite deadlines.

Deleted the redundant raw inventory (**0.289 MiB allocated**) and empty owned
scratch directory. Applesauce **0.5.30 / verified LZFSE** saves **0.141 MiB**
on the completed report; SHA-256, mode, owner and original xattrs match,
clone backup removed. Exact paths/rebuild/rollback: `lang47/{cleanup,compression}.json`.
Local APFS verified by mount; DiskManagement unavailable. Total **0.430 MiB**
savings are operation/path footprints, separate from concurrent checkout
changes. Other lanes, shared artifacts, corpus bindings and `29` preserved.

Final allocated/apparent GiB: checkout **22.14/39.39**, `.reference` **10.93/24.95**, `target` **9.74/12.71**; **72.10 GiB free**. All ceilings pass (`lang47/disk-final.json`; six independent scans, including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L46: scope handoff; no new implementation work (2026-10-25)

Read the full brief through **Update 33**, newest L45/L44/L43/F80/I39 reports
and messages, STATUS and the required workflow. Updates **31–32** supersede
Update 26. Fresh inventory: **673 added / 544 named / 129 without** across
the repository; Lane L unchanged at **109/111 named, two uncovered**. No
runtime changes, new observations, exclusions or commits.

The two residuals remain **TestNativeRasterAdmissionAndCancellation** (three
typed-nil Go image pointers) and **TestNativeGIFLoopCloneAdmission** (self-cyclic
scene). Original inputs cannot be represented by current owned Rust types;
L39 retains source mappings and unsupported witnesses. No repeat investigation
or substitute inputs. Scope reassignment requested. Further driver runs under
this unchanged assignment have no actionable implementation work.

Required e2e guard: **933 SVG + 11 expected errors**, all **966 classifications
unchanged**, **fixed 0 / regressed 0** against L45. Frozen F79 CLI hash verifies
before/after; no builds, Go captures or native suites. Other tiers inherited.
Evidence/reproduction: **.reference/codex-runs/report-lang46.md** and
**lang46/{scope,delta,e2e-run}.json**, complete e2e report, standard delta log
and check.py. Streaming **1 GiB total / 128 MiB file / 20 GiB free** limits
and finite deadlines.

Removed redundant raw inventory (**0.289 MiB allocated**) and empty owned
scratch directory. Verified Applesauce **0.5.30 / LZFSE saves 0.141 MiB** on
the completed report; hashes, mode, owner and original xattrs preserved,
clone backup removed. Exact paths/rebuild/rollback: **lang46/{cleanup,compression}.json**.
Total **0.430 MiB** savings are operation/path footprints during concurrent
work. Local APFS verified; DiskManagement unavailable. Other lanes, shared
artifacts, hard links, corpus bindings and `29` preserved.

Final allocated/apparent GiB: checkout **22.14/39.38**, `.reference` **10.93/24.95**, `target` **9.74/12.71**; **72.48 GiB free**. Every ceiling passes (`lang46/disk-final.json`; six independent complete scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L45: no actionable item in the current assignment (2026-10-25)

Re-read the full brief through **Update 33**, newest L44/L43/L42/F80/I39
reports/messages and current STATUS. Updates **31–32** supersede the completed
Update 26 queue. Fresh inventory remains **673 added / 542 named / 131 without**
repository-wide; Lane L remains **109/111 named, two uncovered**. No runtime
changes, observations, exclusions or commits. Supervisor scope reassignment
was requested; repeating the completed library queue or L39 investigation
would add no coverage.

Residuals remain **TestNativeRasterAdmissionAndCancellation** (three typed-nil
Go images) and **TestNativeGIFLoopCloneAdmission** (self-cyclic scene). Current
owned Rust types cannot express the original inputs. L39 retains original
witnesses and source mappings at master **6a7b48378**, raster_test.go:131–168,
raster.go:475–569, gif_loop_test.go:99–111 and gif_loop.go:17–86. No substitute
inputs were added; both functions remain uncovered.

Required full e2e guard: **933 SVG + 11 expected errors**, all **966
classifications unchanged / zero regressions** against L44. Standard delta:
**fixed 0 / regressed 0**. The frozen F79 runner hash verifies before/after.
No builds, Go captures or native suites; other tiers remain inherited.
Evidence: `.reference/codex-runs/lang45/` scope/delta/e2e-run JSON, complete
e2e report, standard delta log and verify.py. Capture budget: **1 GiB total /
128 MiB file / 20 GiB free**, finite deadlines. Handoff: **report-lang45.md**.

Cleanup deletes the redundant raw inventory (**0.289 MiB allocated**) and empty
owned scratch directory. Verified **Applesauce 0.5.30 / LZFSE** saves **0.141
MiB**, preserving report hash/mode/owner/original xattrs; clone backup removed.
Exact paths/rebuild/rollback: `lang45/{cleanup,compression}.json`. Total savings
**0.430 MiB** are operation/path footprints, not net concurrent checkout change.
Local APFS verified by mount; DiskManagement unavailable. Other lanes, shared
artifacts, hard links, corpus bindings and `29` preserved.

Final allocated/apparent GiB: checkout **22.66/39.89**, `.reference` **11.39/25.40**, `target` **9.79/12.77**; **72.10 GiB free**. Every ceiling passes (`lang45/disk-final.json`; six complete independent scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L44: assigned scope remains exhausted (2026-10-25)

Read the complete brief through **Update 33**, newest L43/L42/F80/I39
reports/messages and STATUS. Superseding Updates **31–32** remain the Lane L
assignment. Fresh inventory: **673 added / 542 named / 131 without** across
the repository; Lane L **109/111 named, two uncovered**, unchanged. No new
coverage, runtime edits, exclusions or commits. Further implementation needs
scope reassignment, requested during this run.

The two residuals remain **TestNativeRasterAdmissionAndCancellation** (three
typed-nil Go images) and **TestNativeGIFLoopCloneAdmission** (self-cyclic scene).
L39 retains original witnesses and source mappings; current owned Rust types
cannot express those original inputs. No repeated investigation or replacement
inputs were added. No actionable failing board exists in the assigned queue.

Full required e2e guard repeats **933 SVG + 11 expected errors**, with all
**966 classifications unchanged / zero regressions** against L43. The frozen
F79 executable hash verifies before/after. No builds or new Go/native suites;
other tiers remain inherited. Evidence: `lang44/{scope,delta,e2e-run}.json`,
`e2e/report.json`, standard delta log and `check.py`; handoff:
`.reference/codex-runs/report-lang44.md`. Streaming budget: **1 GiB total /
128 MiB file / 20 GiB free**, finite deadlines.

Removed redundant raw inventory (**0.289 MiB allocated**) and empty scratch
directory. Applesauce **0.5.30 / verified LZFSE** saves **0.141 MiB** on the
completed e2e report, preserving hash/mode/owner/original xattrs; clone backup
removed. Exact paths/rebuild/rollback: `lang44/{cleanup,compression}.json`.
Other lanes, shared artifacts, original corpus bindings and `29` preserved.
Final allocated/apparent GiB: checkout **22.65/39.88**, `.reference` **11.44/25.45**, `target` **9.74/12.72**; **72.10 GiB free**. All ceilings pass (`lang44/disk-final.json`; six independent complete scans, including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L43: scope inventory and regression handoff (2026-10-25)

Read the full brief, including its initial Update and Updates through **33**,
newest L42/F80/I39 reports/messages and current STATUS. Updates **31–32**
supersede Update 26. Fresh inventory: **673 added / 542 named / 131 without**
repository-wide; Lane L remains **109 named / 111 assigned / two uncovered**.
The repository increase belongs to other shared observations. No new coverage,
runtime edits, exclusions or commits; other lanes and `29` preserved.

Residuals remain **TestNativeRasterAdmissionAndCancellation** (three typed-nil
Go image pointers) and **TestNativeGIFLoopCloneAdmission** (self-cyclic scene).
L39 retains original unsupported witnesses and source mappings. Current owned
Rust types cannot express those original inputs. No repeat investigation or
substituted input; both functions remain uncovered. Further development needs
a supervisor scope reassignment, requested during this run.

Required full e2e guard: **933 SVG / 11 expected errors**, all **966
classifications unchanged / zero regressions** against L42. Used unchanged,
hash-bound frozen F79 CLI; no builds, new Go captures or native suites.
Other tiers remain inherited. Evidence: `lang43/{scope,delta,e2e-run}.json`,
`e2e/report.json`, standard delta log and `check.py`; handoff:
`.reference/codex-runs/report-lang43.md`. Streaming limits: **1 GiB total /
128 MiB file / 20 GiB free**, finite deadlines.

Cleanup removes redundant raw inventory (**0.289 MiB allocated**) and empty
scratch directory. Verified Applesauce **0.5.30 / LZFSE** saves **0.141 MiB**
on the completed report; SHA-256, mode, owner and original xattrs preserved,
clone backup removed. Total operation/path savings: **0.430 MiB**. Exact paths,
replacement/rebuild records and rollback: `lang43/{cleanup,compression}.json`.
No shared artifact or retained corpus input/output removed or rewritten.

Final allocated/apparent GiB: checkout **21.93/39.18**, `.reference` **10.72/24.74**, `target` **9.74/12.71**; **72.86 GiB free**. All ceilings pass (`lang43/disk-final.json`; six complete independent scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L42: assigned scope exhausted (2026-10-25)

Read the full brief through Update 33, including its initial Update, newest
L41/L40/I38/F79 reports/messages and STATUS. Updates 31–32 supersede the
completed Update 26 queue. Fresh inventory: **673 added / 541 named / 132
without** repository-wide; Lane L remains **109 named / 111 assigned / two
uncovered**. Other shared observations account for the repository increase.
No runtime changes, new observations, exclusions or commits.

The residuals remain **TestNativeRasterAdmissionAndCancellation** (typed-nil
Go images) and **TestNativeGIFLoopCloneAdmission** (cyclic scene). Current
owned Rust types cannot express their original inputs. L39 retains original
witnesses and source mappings; this run does not repeat the investigation.
Scope reassignment requested; no dependent work started without an answer.

Required full e2e guard: **933 SVG / 11 expected errors**, all **966
classifications unchanged / zero regressions** against L41, using the
unchanged hash-bound frozen F79 CLI. No builds, new Go captures or native
suites; other tiers remain inherited. Evidence and reproduction:
`.reference/codex-runs/report-lang42.md`, `lang42/{scope,delta,e2e-run}.json`
and `e2e/report.json`. Streaming budgets: 1 GiB total / 128 MiB file /
20 GiB free, finite deadlines. Only this section and ignored run evidence
changed; other lanes preserved.

Redundant raw inventory and empty scratch directory removed; completed e2e
report receives verified Applesauce LZFSE compression with hash, mode, owner
and original xattrs preserved, clone backup removed. Exact paths/replacements
and rollback: `lang42/{cleanup,compression}.json`.

Cleanup removes **0.289 MiB allocated**; verified compression saves **0.141 MiB**, **0.430 MiB total**. Savings are operation/path footprints; concurrent checkout changes are not attributed to this run.

Final allocated/apparent GiB: checkout **21.93/39.17**, `.reference` **10.72/24.74**, `target` **9.74/12.71**; **72.10 GiB free**. All ceilings pass (`lang42/disk-final.json`; six independent complete scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L41: unchanged assigned scope (2026-10-25)

Re-read the full brief through **Update 33**, including its initial Update,
newest L40/I38/F79 reports/messages and STATUS. Updates **31–32** remain the
latest Lane L assignment. Fresh inventory confirms **111 assigned image
functions / 109 named / two uncovered**, unchanged; repository inventory is
**673 added / 521 named / 152 without**. No runtime changes, new observations,
exclusions or commits. Further Lane L work needs a supervisor reassignment.

The residual functions remain **TestNativeRasterAdmissionAndCancellation**
(three typed-nil Go image pointers) and **TestNativeGIFLoopCloneAdmission**
(a self-cyclic scene node). Original inputs cannot be expressed by the current
owned Rust texture/tree types. L39 retains source mappings and original
input/output evidence; this run does not repeat its investigation or replay.
Both functions stay visibly uncovered.

The required full e2e guard repeats **933 SVG / 11 expected errors**, all
**966 classifications unchanged**, **zero regressions** against L40, using
the unchanged hash-bound frozen F79 CLI. No build, new oracle capture or native
suite ran. Full isometric SVG/PNG and native-suite results remain inherited.
Evidence: `.reference/codex-runs/lang41/{scope,delta,e2e-run}.json`,
`e2e/report.json`, standard delta log and `check.py`; handoff:
`report-lang41.md`. Capture limits: **1 GiB total / 128 MiB file / 20 GiB
free**, finite deadlines. Only this STATUS section and ignored run evidence
are new; other lanes' paths remain intact.

Cleanup retires this run's redundant raw inventory and empty scratch folder
(`cleanup.json`). Completed e2e report receives verified Applesauce 0.5.30
LZFSE compression with SHA-256, mode, owner and original xattrs preserved;
clone backup removed (`compression.json`). No shared executable/cache or
retained corpus input/output removed. Final measured savings and complete
disk audit are recorded here.

Cleanup removes **0.285 MiB allocated**; verified compression saves **0.141 MiB**, **0.426 MiB total**. These are this run's operation/path savings; concurrent checkout growth is not attributed to Lane L.

Final allocated/apparent GiB: checkout **22.53/39.65**, `.reference` **11.28/25.21**, `target` **9.77/12.71**; **71.72 GiB free**. Every ceiling passes (`lang41/disk-final.json`; six independent complete scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow the final audit.

### Lane L40: completed-scope handoff (2026-10-25)

Read the complete brief, including the initial Update and all Updates through
**33**, newest L39/F79/I38 reports/messages and current STATUS. Updates
**31–32** supersede Update 26. Fresh inventory confirms **111 assigned
master-added image functions, 109 with named observations and two uncovered**;
no new actionable Lane L item appeared. Whole-repository raw inventory is
**673 added / 521 named / 152 without**. This run adds no coverage, exclusions,
runtime changes or commits, and leaves the other lanes' work intact.

The larger residual remains **TestNativeRasterAdmissionAndCancellation**:
three typed-nil image interfaces have no live native texture value. The other
is **TestNativeGIFLoopCloneAdmission**, whose original self-cyclic scene
cannot be represented by the owned native tree. Source mappings and original
inputs remain in L39; its verifier passes again, including all original
admission input/output hashes and the **9 matching / three unsupported**
retained observations. These are verification of existing evidence, not a
new native replay. Both functions stay visibly uncovered. Further Lane L
development needs a supervisor scope reassignment; no duplicate investigation
or replacement test inputs were introduced.

The required full e2e guard repeats **933 SVG / 11 expected errors**, all **966 classifications unchanged**, **zero regressions** against L39. Used the unchanged hash-bound frozen F79 CLI; no builds, Go captures or native suites ran. L39's 40 native / 38 harness passes and full isometric SVG 870 / PNG 20 remain inherited, not rerun. Evidence: `.reference/codex-runs/lang40/{scope,delta,verification}.json`, e2e report/logs, `inherited-verification.log` and `finish.py`; handoff: `report-lang40.md`. Budgets: 1 GiB total / 128 MiB file / 20 GiB free, finite deadlines.

Cleanup removes the redundant raw inventory (**0.285 MiB allocated**) and empty scratch directory; scoped counts/rebuild command remain (`cleanup.json`). Verified Applesauce 0.5.30 LZFSE compression of the completed e2e report saves **0.141 MiB**; SHA-256, mode, owner and original xattrs match, clone backup removed. Exact target and rollback in `compression.json`; no retained corpus input/output or shared cache/executable removed, no hard links broken or bulk moved.

Final allocated/apparent GiB: checkout **21.88/39.02**, `.reference` **10.70/24.64**, `target` **9.71/12.66**; **82.40 GiB free**. Every ceiling passes (`lang40/disk-final.json`; six complete independent scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L39: remaining Lane L input-model boundaries (2026-10-25)

Re-read the full brief through **Update 33**, including its initial Update,
newest L38/I37/F78 reports/messages and STATUS. Updates **31–32** supersede
the old Update 26 queue. No runtime changes, commits or new coverage claims.

Started with the larger residual: **TestNativeRasterAdmissionAndCancellation**
has three unsupported typed-nil image calls. Replayed its exact retained
L32 inputs: **9 matching / 3 unsupported**, unchanged. All three non-nil
Go interfaces hold nil image pointers; Rust's `Option<Rc<Texture>>` represents
either an absent texture or a live value. Substituting absent or malformed
storage would change the original input. Source: **raster_test.go:131–168**,
**raster.go:475–569**, native **mesh.rs:84–90,156**.

The other function, **TestNativeGIFLoopCloneAdmission**, passes the same
self-cyclic scene node twice, including with a canceled context. Owned
`Vec<Node>`/boxed mask roots cannot form that graph. A canceled valid tree
does not qualify those original calls. Source: **gif_loop_test.go:99–111**,
**gif_loop.go:17–86**, native **d2-rs-scene/src/scene.rs:201–213** and
**img/gif_loop.rs**. Update 30 permits skipping clearly Go-internal contracts;
both functions remain visibly **uncovered**, with no inventory exclusions.
No further actionable runtime mismatch was established within Lane L's scope.

**40 tests pass on the existing hash-bound warm native runner**, with three
ordinary external-capture tests ignored; the admission replay executes its
normally ignored transport explicitly. **38 raster-harness checks pass.**
E2e remains **933 SVG / 11 expected errors**, all **966 classifications
unchanged**, **zero regressions** against L38. Used frozen F79 CLI; no build
or new oracle capture. Isometric SVG **870**, PNG **20** and other full tiers
remain inherited. Retained L38 closure verifies **1,267 SVG input bindings /
1,008 expected outputs**; original L32 capture hashes and complete new raw
pixel inventory verify after compression.

Fresh raw inventory: **673 added / 501 named / 172 without**. Lane L remains
**2 uncovered out of 111 assigned**; this run adds zero named observations.
Evidence: `.reference/codex-runs/lang39/{source,residuals,admission,scope-final,
delta}.json`, `audit.py`, `verify.py`, test/guard logs and e2e report.
The scope is ready for supervisor reassignment; original unsupported inputs
remain available. Replay budgets: **1 GiB total / 128 MiB file / 20 GiB free**,
finite deadlines. Other lanes, shared caches/executables and `29` preserved.

Cleanup removes one redundant inventory / **0.28 MiB allocated** plus two empty scratch directories (`cleanup.json`); scoped counts and the raw-inventory rebuild command remain. Verified **Applesauce 0.5.30 / LZFSE** saves **0.16 MiB across three retained artifacts**, preserving SHA-256, modes, owners and original xattrs. Clone backups removed; rollback and exact paths: `compression.json`, `applesauce decompress --verify -- <path>`. Local APFS verified; DiskManagement unavailable. No hard links, active files, shared binaries or caches rewritten; no bulk moved.

Final allocated/apparent GiB: checkout **21.90/39.02**, `.reference` **10.70/24.64**, `target` **9.71/12.66**; **81.63 GiB free**. Every ceiling passes (`lang39/disk-final.json`; six complete independent scans including hidden paths/.git and every apparent hard link). Measurements are non-atomic during concurrent work; other lanes' changes are not attributed as L39 savings. No builds or parity captures follow the final audit.

### Lane L38: explicit icon resolver policy and physical/structured vectors (2026-10-25)

Read the complete brief through **Update 33**, newest reports/messages and
STATUS, including F78/I37; followed the superseding Updates 31–32 scope.
Original **TestSurfaceIconRasterAndExplicitLocalResolver** exposed a real
failure: sharing the default painter's resolver allowed a local PNG read that
Go denies. `surface_icons.rs` now preserves imageasset's default denied
local-file policy. Caller-supplied rooted resolvers still render identical
pixels. Source: **surface_icons.go:59–90; surface_icons_test.go:70–130**.

Also captured **TestNativeVectorCaptureCoversPhysicalAndStructuredSurfaces**
for four original boards. A shared `build_native_scene_with_options` entry
preserves explicit caller density/framing options, including density zero;
ordinary render callers retain their derived options. The vector calls
already match; this entry refactor adds no claimed vector runtime gain.
Source: **geometry.go:431–478; vector_surface_test.go:337–379**.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / observations | **1/2 → 2/2; 10/12 → 12/12** | **0** |
| Full isometric SVG / source errors | **870 / 138 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

**48 native + three harness tests pass**, including all **20 PNG goldens**.
The retained fixture repeats after compression. Rebuilt frozen executables
repeat every one of 1,008 isometric and 966 e2e classifications. Full SVG
capture uses 1,011,162,188 bytes within its 1 GiB budget. Other full tiers
remain inherited; no golden gain or full API parity claim.

Every original Go assertion stays active. Replay compares exact errors,
real local-file admission, caller file bytes/limits, complete scene/source
values, ordered meshes, material/group/owner/vector sharing, all texture
pixel hashes and all 123 retained vector fragments. Expected values never
enter native requests. JSON bridges compile only in tests. Internal resolver
allocation counts and unobserved caller assertions remain outside this
projection. Initial observer/compile/shared-hook-lock setup is excluded;
the two failed observations share the single erroneous local-read cause.

Fresh inventory: **673 added / 499 named / 174 without**.
**Lane L four → two uncovered**, out of 111 assigned; no exclusions.
`TestNativeGIFLoopCloneAdmission` contains a cyclic Go pointer graph and
`TestNativeRasterAdmissionAndCancellation` three typed-nil image interfaces.
Those exact inputs have no owned native representation; partial coverage and
substitute malformed values do not qualify the complete original functions.

Evidence: `.reference/codex-runs/lang38/` original `artwork-qualified`,
`artwork-{negative,final}.json`, complete `negative-icon-witness.json`,
`runtime-fix.patch`, scoped source hashes, deltas, scope, guard/test logs and
`verify.py`; alias **isometric-artwork-api.json**. Original input/output and
fixture closure, source restoration, 1,267 SVG input bindings, 1,008 expected
outputs and all compressed hashes/modes/owners verify after cleanup. No
commits or full concurrent source freeze. Streaming limits: **1 GiB per
capture / 128 MiB file / 20 GiB free**, finite deadlines.

Cleanup removes **1,753 files / 1,219.39 MiB allocated path footprint**, mostly this run's
successful SVG copies replaced by exact pinned goldens, obsolete observer
capture and four idle ephemeral runners. Negative icon witness and complete
final API output remain; SVG error bodies and all source inputs remain.
`cleanup.json`, `svg-inputs.json` and `runner-retirement.json` explicitly bind
replacements/rebuilds and retire historical executable paths. Shared Cargo
outputs/caches are untouched. Verified **Applesauce 0.5.30 / LZFSE** saves
**45.66 MiB across 10 retained files**; SHA-256, modes, owners and original xattrs
verify, clone backups removed. Exact paths/rollback: `compression-evidence.json`,
`applesauce decompress --verify -- <path>`. Local APFS verified;
DiskManagement unavailable. No active/shared executable compressed, hard link
broken or bulk moved. Savings are path/operation footprints during concurrent
work, not net volume attribution.

Final allocated/apparent GiB: checkout **22.31/39.37**, `.reference` **11.09/25.01**, `target` **9.75/12.65**; **82.40 GiB free**. Every ceiling passes (`lang38/disk-final.json`; six complete independent scans, including hidden paths/.git and every apparent hard link). Non-atomic during concurrent work. No builds or parity captures follow this final audit.

### Run 79: nonblocking regular-file import opens (2026-10-25)

Continued Lane F after reading the complete brief through **Update 33**, newest
reports/messages and STATUS, including arriving I37/L38. Fixed the original
**TestTrackedFSRejectsFIFOWithoutBlocking** contract at the CLI import boundary:
the retained `...@import.d2` FIFO witness timed out before; it now returns Go's
complete regular-file error immediately. Rust previously recorded the dependency
before `fs::read`. It now opens through the existing descriptor-validated
`ImageLocalFiles::Unrestricted` policy, then records success and supplies the
opened reader. Source: **d2cli/watch.go:740–752**, `lib/localfile/{localfile,
open_unix}.go`, master **6a7b48378**. No path-before-open substitute.

| Tier | Before → final | Regressions |
|---|---:|---:|
| CLI FIFO import witness | **timeout → exact error match** | **0** |
| Original trackedFS functions / calls | **newly measured: 2/2 / 6/6** | **0** |
| E2e SVG / expected errors | **933 / 11 unchanged** | **0** |
| CLI complete artifacts / goldens, release/shared clock | **67 / 51 unchanged** | **0** |
| cli-diff, shared clock | **119/119 unchanged** | **0** |

**247 native + five harness tests pass**. The native regression test exercises
the real import opener with regular, missing, directory and FIFO inputs and
checks that rejected opens add no dependency. The original rooted/denied/
unrestricted policy assertions remain active through an input-only `cfg(test)`
transport. Complete errors, typed denial and opened-path lists compare exactly;
Go's nil append-only list is represented explicitly. Close-only original
assertions stay active; arbitrary close failures and other platform-specific
special files are outside this projection. Initial observer count/nil-list and
native test setup errors are excluded from runtime gains. Other full tiers are
inherited; no new golden or full filesystem/platform parity claim.

Evidence: **.reference/codex-runs/run79/** `before/after-fifo.json`,
`tracked-sealed/`, `tracked-sealed.json`, complete CLI/e2e captures, deltas,
scoped source hashes/patch and `verify.py`. Alias: **tracked-fs-api.json**.
Original assertions, capture bindings, Go source restoration, compressed bytes
and modes verify. Capture limits: **1 GiB total / 128 MiB file / 20 GiB free**,
finite suite and native deadlines. No commits; other lanes and `29` preserved.
Fresh inventory: **673 / 501 named / 172 without**; **Lane F 96 → 94**,
**CLI 50 → 48** uncovered. Next prefer the original paged bounded-writer/
admission contract; many remaining CLI names repeat qualified board behavior.

Cleanup removed **4,418 files / 614.91 MiB allocated path footprint**:
three idle superseded Cargo executables plus depfiles, observer setup captures,
the recreated FIFO and the verified idle project Go action cache. Exact paths,
hashes, replacements and historical runner retirements: `run79/cleanup.json`,
`native-runner-retirement.json`, `retired-variant-verification.json`. The old
variant audit finds only historical compression bindings, now explicitly
retired. Original corpus inputs/outputs remain. **Applesauce 0.5.30 / LZFSE**
saves **35.06 MiB / 18 files**, including the idle qualified CLI; SHA-256,
modes, owners and original xattrs verify; clone backups are removed. Rollback:
`applesauce decompress --verify -- <path>`; exact targets: `compression.json`.
Local APFS verified; DiskManagement unavailable. No active executable or shared Cargo output
compressed, hard link broken or bulk moved. All **1,707 retained input/output
bindings** verify after cleanup/compression.

Final allocated/apparent GiB: checkout **21.95/39.06**, `.reference`
**10.75/24.68**, `target` **9.71/12.66**; **81.63 GiB free**. Every ceiling
passes (`run79/disk-final.json`; six complete independent scans, including
hidden paths/.git and every apparent hard link). Non-atomic during concurrent
work. No builds or parity captures follow the final audit.

### Run 78: original isometric publication and output preservation (2026-10-25)

Re-read the full brief through **Update 33**, newest reports/messages and
STATUS. Continued Lane F after run 77. Four original functions now have
named native observations: **TestIsometricMultiboardPublicationIsTransactional**,
**TestIsometricImageAtomicReplacement**, **TestIsometricSVGFailurePreservesOutput**
and **TestIsometricPagedFailurePreservesOutput**. All **20 calls already
pass**; no production algorithm change was needed. The only implementation
addition is a `cfg(test)` input-only bridge to the existing atomic writer.

| Tier | Measured / final | Regressions |
|---|---:|---:|
| Original functions / calls, explicit release stamp | **4/4 / 20/20 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

Every original Go assertion remains active. File/stdout failures preserve
reviewed output; cancellation preserves the old image; successful atomic
replacement preserves private modes and leaves no temporary files.
Multiboard capture covers a failed child, stale generated-file retirement,
unrelated-file preservation and staging cleanup. Complete ordered before/
after inventories, bytes, permissions, links and stdout compare at identical
paths; atomic errors compare exactly. Source: `d2cli/isometric_image.go:
294–339`, `board_output*.go`, and the original image/integration/SVG/paged
test functions. CLI error text beyond their failure assertions is outside
this projection.

The ordinary source stamp remains visibly **3/4** in strict artifact
comparison: transaction SVGs contain Go's `v0.8.1-HEAD` versus Rust's release
`v0.9.0`, with corresponding manifest hashes. Both implementations pass all
four original functions in that profile. The qualified profile explicitly
stamps Go `v0.9.0` with release ldflags and compares full SVG/manifest bytes;
no normalization or runtime gain is claimed. `--source-version` reproduces
the separate source-profile measurement. Initial duplicate temporary-name
observer setup is excluded from native results.

**246 native + six harness tests pass**; the new normally ignored atomic
transport executes explicitly through the original Go test. All **966 e2e
classifications** repeat on the unchanged frozen run77 CLI. Production code
is unchanged, so other full scoreboards remain inherited; no new golden gain
or full filesystem/platform parity claim. No commits; other lanes and `29`
preserved. Streaming limits: **1 GiB total / 128 MiB file / 20 GiB free**,
600-second suites and finite native invocations.

Evidence: `.reference/codex-runs/run78/` source-profile and final captures,
`deltas.json`, `source.json`, `verify.py`, native/harness logs, original
source restoration and e2e guards. Alias: **isometric-publication.json**.
The verifier checks every retained call input/expected-output and the
**1,294 original run77 capture/e2e input/output bindings** after cleanup.
No full source freeze or executable body was added.

Fresh inventory: **673 added / 476 named / 197 without**.
**Lane F 100 → 96; CLI 54 → 50 uncovered**. No exclusions. Prefer the
original bounded-writer/admission and local-file/FIFO contracts next; many
remaining CLI functions repeat qualified whole-board behavior.

Cleanup removed **4,986 files / 869.71 MiB allocated path footprint**,
mostly the verified idle Go action cache and superseded setup captures.
Verified **Applesauce 0.5.30 / LZFSE** saves **27.00 MiB / 31 files**;
SHA-256, modes, owners and original xattrs verify, and clone backups are
removed. Local APFS verified; DiskManagement unavailable. Exact paths and
rollback: `run78/{cleanup,compression}.json`, `applesauce decompress --verify
-- <path>`. No shared/active executable or hard link compressed; no bulk moved.
Older frozen CLIs remain bound by current baselines; no new executable copy
was created. `warm-runner-retirement.json` explicitly retires the old mutable
Cargo test-runner hash; original input/output bindings still verify.

Final allocated/apparent GiB: checkout **22.43/39.43**, `.reference` **11.07/24.95**, `target` **9.86/12.77**; **86.98 GiB free**. Every ceiling passes (`run78/disk-final.json`; six independent complete scans, hidden paths/.git and every apparent hard link included). Measurements are non-atomic during concurrent work. No builds or parity captures follow the audit.

### Lane I37: original hierarchy-support and SVG-lighting contracts (2026-10-25)

Read the full brief through **Update 33**, newest reports/messages and STATUS;
followed superseding Updates 31–32. Qualified **21 original functions / 358
input-only observations**, covering all 14 hierarchy board/solid/support/terrace
functions and seven SVG-lighting functions. Every original Go assertion stays
active. **All observations already match; no production algorithm fix.**
Sources: `hierarchy_{boards,support,headers}.go`, `native_shapes.go`,
`svg_lighting.go` and `svg.go` at **6a7b48378**.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / observations | **21/21 / 358/358 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

**48 native + five harness tests pass**, including all **20 PNG goldens** and
selected whole-board SVG/assets. The final frozen runner repeats all 358
observations; the complete fixture also passes when read after compression.
Every one of 966 e2e classifications repeats on the frozen I25 CLI. Full
isometric SVG **870/870** and other full tiers remain inherited; no golden
gain or full API parity claim. New JSON bridges compile only under `cfg(test)`.

Complete ordered meshes, material/group/owner identities, every texture pixel
hash, support/drop and rich/face budgets, source nodes/boards, hierarchy plans,
lighting coefficients/gradient patches and complete shadow SVG bytes compare.
Full retained vector document trees/assets and uncaptured caller/raster
assertions remain outside this projection. The typed bridge preserves Go's
input nil node-ID marker for unchanged empty Rust Vecs; nonempty native
contents always serialize. Four initial wire-only mismatches and count/observer
setup are excluded from runtime gains. An in-progress Lane L38 explicit-icon
test initially failed; the final full library retry includes it and passes.
No commits or full concurrent checkout source freeze.

Fresh inventory including F78: **673 added / 497 named / 176 without**.
**Lane I 82 → 61 image functions**, plus the scene URL/Userinfo handoff:
**83 → 62 total**. No applicability exclusions. Next prefer SVG seam/coverage,
ground-receiver and Markdown-card contracts; put actual failures first.

Evidence: `.reference/codex-runs/iso37/` `qualified-source`,
`qualified-{before,final}.json`, input-only final requests, `delta.json`,
native/harness logs and `verify.py`; alias **isometric-hierarchy-api.json**.
Original input/expected-output closure, source restoration, fixture membership
and compressed hashes/modes/owners verify. Streaming capture/replay limits:
**1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.

Cleanup retires **204 files / 285.32 MiB allocated path footprint**, including
superseded capture/replay outputs and three transient runners. Complete
qualified inputs/expectations remain. Verified **Applesauce 0.5.30 / LZFSE**
saves **28.79 MiB / eight retained files**, preserving SHA-256, modes, owners
and original xattrs; clone backups are removed. Exact paths/replacements:
`iso37/{cleanup,compression,runner-retirement}.json`; rollback:
`applesauce decompress --verify -- <path>`. Local APFS verified; DiskManagement
unavailable. No shared executable/cache removed or compressed, hard links
broken or bulk moved. Savings are operation/path footprints.

Final allocated/apparent GiB: checkout **23.12/40.19**, `.reference` **11.79/24.90**, `target` **9.86/12.77**; **85.07 GiB free**. Every ceiling passes (`iso37/disk-final.json`; six complete independent scans, hidden paths/.git and every apparent hard link included). Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane I36: original structured-node and sequence-route contracts (2026-10-25)

Continued the brief through **Update 33**, following superseding Updates
31–32. Captured all original assertions in six `native_structured_test.go`
functions, six `sequence_routes_test.go` functions and
`svg_precision_test.go:198–224`'s structured-row rounding regression.
**13/13 functions / 50/50 input-only observations already match**; no
production algorithm fix was needed. The new bridge and JSON construction
compile only in tests. Explicit concrete-paint serialization preserves every
text-run field across Rust enum and Go interface representations.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / observations | **13/13 / 50/50 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

**46 native + five harness tests pass**, including all **20 PNG goldens** and
the selected whole-board SVG/asset tests. The rebuilt frozen runner repeats
all 50 calls; the complete retained fixture passes after compression. All
966 e2e classifications repeat on the frozen I25 CLI. Full isometric SVG
**870/870** and other full tiers remain inherited; no new golden gain or
full API parity claim.

Replay compares ordered meshes, material/group/owner identities, every
texture pixel hash, source nodes/edges, rich/face budgets, complete text-cell
runs/viewports, routes/errors, caption surfaces, packet tracks/materials and
placer state. Full document trees/assets and uncaptured caller/raster
assertions remain outside this projection. Every original Go assertion
stays active. Initial count discovery, enum-tag bridge setup, a concurrent
Lane L bridge compile failure and a rejected mutable-runner hash race are
excluded. A frozen replay resolves the race. No commits or full concurrent
source freeze.

Fresh inventory: **673 added / 469 named / 204 without**.
**Lane I 95 → 82 image functions**, plus the existing scene URL/Userinfo
handoff: **96 → 83 total**. No exclusions. Next: original SVG lighting/seam
and hierarchy support contracts; prioritize any observed failure.

Evidence: `.reference/codex-runs/iso36/` `combined-source`,
`combined-{before,sealed}.json`, input-only final requests, `delta.json`,
native/harness logs and `verify.py`; alias **isometric-structured-api.json**.
Original inputs/expected outputs, fixture membership, Go restoration and
compressed hashes/modes/owners verify. Capture/replay streaming limits:
**1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.

Cleanup removes **137 files / 161.16 MiB allocated path footprint**;
verified **Applesauce 0.5.30 / LZFSE** saves **29.50 MiB / 33 retained files**.
SHA-256, modes, owners and original xattrs verify; clone backups are removed.
Exact paths/replacements: `iso36/{cleanup,compression,runner-retirement}.json`;
rollback: `applesauce decompress --verify -- <path>`. Local APFS verified;
DiskManagement unavailable. No shared executables/caches removed or compressed,
no hard links broken and no bulk moved. Savings are path/operation footprints.

Final allocated/apparent GiB: checkout **22.80/39.72**, `.reference` **11.45/25.29**, `target` **9.92/12.79**; **85.83 GiB free**. Every ceiling passes
(`iso36/disk-final.json`; six complete independent scans), including hidden
paths/.git and every apparent hard link. Non-atomic during concurrent work.
No builds or parity captures follow the final audit.

### Run 77: board publication rollback and mutation-time checks (2026-10-25)

Re-read the full brief, including its initial Update and Updates through
**33**, newest reports/messages and STATUS; incorporated arriving I35/L36.
Continued Lane F's publication queue. Three original functions exposed real
failures: **TestBoardOutputPublicationRollsBackWholeTransaction**,
**TestBoardOutputPublicationRejectsAncestorSwapWithoutEscapingRoot**, and
**TestBoardOutputPublicationPreservesStaleFileEditedDuringPublish**.

Rust now prepares all backups before mutation, journals installed files and
created/removed directories, publishes the manifest last, and restores the
journal in reverse on failure. It rechecks stale-file identity and bytes
immediately before retirement. Unix mutations resolve from an opened root
through descriptor-relative operations, so the injected symlink swap cannot
redirect publication outside that root. Installed identities protect rollback
from overwriting replacements. Source: **d2cli/board_output_publish.go:
414–661,664–809,831–992**; root handle **117–165**.

| Tier | Before → qualified | Regressions |
|---|---:|---:|
| Original transaction functions / complete observations | **0/3 → 3/3** | **0** |
| Existing publication functions / snapshots | **5/5 / 10/10 unchanged** | **0** |
| Preflight cases | **4/4 unchanged** | **0** |
| Manifest API / CLI witnesses | **16/16 / 7/7 unchanged** | **0** |
| Unicode-collision CLI witnesses | **4/4 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

Every original Go assertion and callback remains active. The cfg(test)-only
native transport calls the original callback synchronously at each production
mutation boundary. Complete returned errors, touched state, callback order,
node inventories, file bytes, permissions and symlink targets compare at
identical paths without normalization. **246 native + 19 harness tests pass**;
both normally ignored API transports execute explicitly through their
harnesses. Four new ordinary tests cover restoration, stale edits, ancestor
swaps and a real source-read failure. All **966 e2e classifications** repeat
on the rebuilt frozen CLI. Other full tiers remain inherited; no new golden
gain, arbitrary concurrent-replacement/rollback-failure qualification or full
publication/platform parity claim. Windows retains a path-based parent
fallback and was not exercised on this Mac.

Evidence: `.reference/codex-runs/run77/` negative/qualified captures,
`runtime-fix.patch`, scoped `source.json`, `deltas.json`, `verify.py`, native
and harness logs and fresh guards. Alias: **board-transaction.json**. The
verifier checks **1,294** original input/expected-output and capture bindings,
Go source restoration and reverse patch applicability. Streaming capture
limits are **1 GiB total / 128 MiB file / 20 GiB free**, with finite deadlines.
No full concurrent checkout freeze or commits. Initial formatting and
superseded successful captures are excluded from final evidence.

Fresh inventory: **673 added / 456 named / 217 without**, including L36.
**Lane F 103 → 100 uncovered; CLI 57 → 54**. No exclusions. Next prefer the
original isometric multiboard transaction, atomic image replacement and
failure-preserves-output contracts over already-proven component helpers.
Stage-root replacement, rollback failures and broader platform behavior
remain unqualified.

Cleanup retires **38 files / 117.47 MiB allocated path footprint**, including
three idle superseded executable variants and redundant successful captures.
Original inputs, complete negative witnesses and qualified expectations
remain (`cleanup.json`; historical executable bindings explicitly retired).
Verified **Applesauce 0.5.30 / LZFSE** saves **34.05 MiB across 42 retained
files**, including the frozen CLI. SHA-256, modes, owners and original xattrs
verify; clone backups are removed. All 42 hashes/modes recheck, the CLI
executes, and the source/capture verifier passes after compression. Exact
targets/rollback: `compression.json`, `applesauce decompress --verify -- <path>`.
Local APFS verified; DiskManagement unavailable. No active/shared executable
was compressed and no bulk moved. Savings are path/operation measurements
during concurrent work.

Final allocated/apparent GiB: checkout **22.76/39.71**, `.reference`
**11.44/25.28**, `target` **9.85/12.76**; **85.83 GiB free**. Every ceiling
passes (`run77/disk-final.json`; six complete independent scans, hidden
paths/.git and every apparent hard link included). Measurements are
non-atomic during concurrent work. Shared reference binaries were reviewed
and retained; no builds or parity captures follow this audit.

### Lane I35: native face cancellation before allocation (2026-10-25)

Read the complete brief through **Update 33**, newest reports/messages and
STATUS; continued the superseding Updates 31–32 Lane I split. Original
**TestNativeFaceTextureBudgetAndCancellation** exposed a missing context
handoff: Rust's face adapter rendered a canceled request and spent **98,576
pixels**. `native_face_layer` now rejects cancellation before building the
face, matching **native_shapes.go:191–199** (`d2scenebuild.Build(b.ctx, ...)`):
complete `native shape face: context canceled`, no texture, zero area and no
pixel spend. No fixture-specific result or tolerance.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / complete observations | **19/20 → 20/20; 186/187 → 187/187** | **0** |
| Whole-board isometric SVG / source errors | **870 / 138 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

Twenty original source-shape, face-ink, page-hinge, geometry and sequence
functions retain every Go assertion. Input-only native replay compares
ordered geometry, shared material/group/owner identities, all texture pixel
hashes, complete source/profile/label values, errors and budgets. Private
animation ranges are explicitly captured. Uncaptured caller/raster assertions
and vector document bodies remain outside this projection; no new golden gain
or full API parity claim. Bridges and JSON construction compile only in tests.

**44 native + six harness tests pass**, including all **20 PNG goldens**.
The final executable repeats all **187** observations; the rebuilt probe
preserves all **1,008 SVG classifications**. All **966 e2e classifications**
repeat on the frozen I25 CLI; other full tiers remain inherited. Scoped
rustfmt/Ruff/Go-format/diff checks pass. Initial count discovery and three
observer-only animation differences are excluded; corrected Go observations
recompare the unchanged native negative output after proving identical inputs.
No commits or full concurrent checkout source freeze.

Fresh inventory including F76: **673 added / 451 named / 222 without**.
**Lane I 115 → 95 image functions**, plus one scene URL/Userinfo handoff:
**116 → 96 total**. No exclusions. Next prefer the six original structured
node tests and remaining sequence contracts, fixing measured failures first.

Evidence: `.reference/codex-runs/iso35/` complete original `shape-complete-source`,
`shape-{before,sealed}.json`, input-only request shards, complete cancellation
negative witness, `runtime-fix.patch`, `delta.json`, native/harness logs and
`verify.py`. Alias: **isometric-shape-api.json**. Input/expected-output closure,
fixture membership, negative-source reconstruction, Go restoration, 1,267
pinned SVG inputs and every source-error witness verify after cleanup and
compression. Streaming **1 GiB total / 128 MiB file / 20 GiB free** guards and
finite deadlines apply.

Cleanup removes **1,919 files / 1,159.71 MiB allocated path footprint**;
verified **Applesauce 0.5.30 / LZFSE** saves **32.44 MiB across 49 retained files**.
SHA-256, modes, owners and original xattrs verify; clone backups are removed.
Exact paths/replacements/rollback: `iso35/{cleanup,compression}.json`,
`applesauce decompress --verify -- <path>`. Local APFS verified;
DiskManagement unavailable. No shared executable/cache removed or compressed,
no hard links broken and no bulk moved. Savings are operation/path footprints.

Final allocated/apparent GiB: checkout **24.15/40.66**, `.reference` **12.73/26.14**, `target` **9.95/12.86**; **84.69 GiB free**. Every ceiling passes
(`iso35/disk-final.json`; six complete independent scans). Hidden paths/.git
and every apparent hard link are included. Measurements are non-atomic during
concurrent work. No builds or parity captures follow this audit.

### Run 76: whole-tree board publication preflight (2026-10-25)

Re-read the full brief, initial Update and Updates through **33**, newest
F75/I34/L35 reports/messages and STATUS; continued Lane F's CLI queue.
Fixed **TestMultiboardOutputRejectsPreexistingSymlink** and
**TestMultiboardOutputRejectsPreexistingSymlinkAncestor**. Rust checked
destination types while publishing; Go preflights the complete staged tree.
Two companion boards with a late symlink demonstrated leftover `index.svg`
and `index.png` in Rust. `collect_tree` now rejects every existing symlink
or incompatible destination type before creating directories or installing
files, in the source's lexical recursive order. Complete diagnostics use
Go quoting. Source: **d2cli/board_output.go:433–482** (`preflightMerge`).

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / all preflight invocations | **0/2 → 2/2; 0/4 → 4/4** | **0** |
| Existing publication functions / complete snapshots | **5/5 / 10/10 unchanged** | **0** |
| Manifest API / CLI witnesses | **16/16 / 7/7 unchanged** | **0** |
| Unicode-collision CLI witnesses | **4/4 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

All original Go assertions remain active. Both implementations use identical
paths; complete returned errors, before/after node inventories, regular-file
bytes, permission modes and symlink targets compare without normalization.
Rejected exports preserve existing trees and leave no staging directories.
PNG cases qualify rejection only. **242 native + 13 harness tests pass**;
the ordinarily ignored manifest transport replay runs explicitly through its
harness. A new native regression checks that early staged directories and
files remain unpublished for both a late symlink and a late type conflict.
All **966 e2e classifications** repeat on the frozen corrected CLI. Other
full tiers remain inherited; no new golden gain or full publication/API
parity claim. Concurrent ancestor swaps, descriptor-relative publication and
rollback after visible mutations remain open.

Evidence: `.reference/codex-runs/run76/` negative/final preflight captures,
`{deltas,source,negative-source}.json`, `runtime-fix.patch`, `verify.py`,
native/harness logs and fresh e2e/publication/manifest/collision guards.
Alias: **board-preflight.json**. Verification checks original source
restoration, all 16 negative/final capture records and scoped source hashes.
Limits: streaming **1 GiB total / 128 MiB file / 20 GiB free**, finite
deadlines. Initial xmain wrapper observer setup is excluded; the qualified
observer captures the actual Go Run return, preserving full diagnostic text.
Shared Cargo waits and initial tooling checks are excluded from parity
evidence. No full concurrent checkout freeze or commits.

Fresh inventory: **673 added / 431 named / 242 without**. **Lane F
105 → 103 uncovered**, including CLI **59 → 57**. No exclusions. Next:
remaining original CLI transaction/rollback contracts, putting observed
failures before further inventory.

Cleanup removes **eight obsolete observer/setup files / 0.30 MiB allocated
path footprint**, preserving qualified negative and final captures
(`cleanup.json`). Verified **Applesauce 0.5.30 / LZFSE** saves **33.71 MiB
across 39 retained files**, including the frozen CLI; SHA-256, modes,
ownership and original xattrs verify, and clone backups are removed. All
39 hashes/modes recheck, the CLI executes, and the capture/source verifier
passes after compression. Exact targets and rollback: `compression.json`,
`applesauce decompress --verify -- <path>`. Local APFS verified; DiskManagement
is unavailable. No shared executable removed/compressed and no bulk moved.
Savings are path/operation measurements during concurrent work.

Final allocated/apparent GiB: checkout **23.16/39.61**, `.reference`
**11.81/25.20**, `target` **9.88/12.75**; **91.93 GiB free**. Every ceiling
passes (`run76/disk-final.json`; six complete independent scans). Hidden
paths/.git and every apparent hard link are included. Measurements are
non-atomic during concurrent work. No builds or parity captures follow this
audit.

### Lane I34: solid cap and caption clipping rounding (2026-10-25)

Read the full brief through **Update 33**, newest reports/messages and STATUS;
continued Lane I's superseding Updates 31–32 file split. Twenty original
functions produce 260 calls. Four calls across three tests exposed two
incorrect ARM64 contraction choices: hexagon ink fitting must round `ny*y`
and `ny*ay`, then fuse `nx*x` and `nx*ax` (**native_solids.go:130**); caption
clipping must round `dz*sin`, then fuse `dx*cos`
(**route_caption_mask.go:39**). Pinned Go 1.27 compiler register/instruction
excerpts establish both choices. Rust now follows them without tolerance.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / complete observed calls | **17/20 → 20/20; 256/260 → 260/260** | **0** |
| Whole-board isometric SVG / source errors | **870 / 138 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

Failing tests: **TestNativeSolidsKeepGeometryAcrossPaintStyles**,
**TestRouteCaptionKnockoutPreservesProjectedDashesAndAlpha**, and
**TestConnectionCaptionGapRevealsUnderlyingSurface**. All original Go
assertions remain active. Input-only native replay compares full ordered
vertices/normals/UVs and triangle metadata, material/group/owner sharing,
complete premultiplied RGBA texture hashes, source values and builder
errors/face budgets. Original caller/raster assertions, arbitrary callbacks
and vector document bodies remain outside this projection. No new golden
gain or full API parity claim. JSON bridges compile only in tests.

**42 native + 3 harness tests pass**, including all **20 PNG goldens**.
The full fresh SVG guard preserves all **1,008 classifications**; all
**966 e2e classifications** repeat on the frozen I25 CLI. Other complete
tiers remain inherited. Scoped rustfmt/Ruff/diff checks pass. The new
compact fixture retains all four failures and triangle ownership metadata.
Initial observer typed-nil setup and adapter compilation attempts are
excluded. Absolute shared Cargo/Go caches; no checkpoint executable copies.

Fresh inventory including F75/L35: **673 added / 429 named / 244 without**.
**Lane I 135 → 115 image functions**, plus one scene URL/Userinfo handoff:
**136 → 116 total**. No exclusions. Next prefer native structured nodes and
remaining source-shape/sequence contracts, putting measured failures first.

Evidence: `.reference/codex-runs/iso34/` original `solid-qualified`,
`solid-{negative,final}.json`, all input-only request shards, four full
negative responses in `negative-witnesses.json`, `runtime-fix.patch`,
compiler excerpts/rebuild commands in `asm/`, and `verify.py`. Alias:
**isometric-solid-api.json**. The verifier checks original source restoration,
exact original input/expected-output closure, fixture membership, negative
source reconstruction, 1,267 pinned SVG inputs, golden hashes and complete
source error witnesses. Captures use streaming **1 GiB total / 128 MiB file /
20 GiB free** guards and finite deadlines; replay is sharded to preserve
those limits. No full concurrent checkout freeze; no commits.

Cleanup retires **1,795 files / 1,353.57 MiB allocated path footprint**;
verified **Applesauce 0.5.30 / LZFSE** saves **155.55 MiB across 69 files**.
Original inputs/expected outputs, all request shards, four complete negative
responses, reverse source patch and complete error witnesses remain. Exact
paths/replacements/rollback: `iso34/{cleanup,compression}.json`. SHA-256,
modes, owners and original xattrs verify; clone backups are removed. No
shared executable/cache removed or compressed, no hard links broken, no bulk
moved. These are path/operation footprints.

Final allocated/apparent GiB: checkout **22.47/38.90**, `.reference` **11.19/24.58**, `target` **9.81/12.67**; **92.70 GiB free**. Every ceiling passes (`iso34/disk-final.json`; six complete independent scans). Hidden paths/.git and every apparent hard link are included. Measurements are non-atomic during concurrent work. No builds or parity captures follow this audit.

### Lane L37: source-panel reservations and caption/artwork calls (2026-10-25)

Continued the superseding **Updates 31–32** assignment after reading the
complete brief through Update 33 and newest lane reports. Three original
functions / **11 native calls already match**, without a production fix:
`TestOutputDensityReservesFollowingSourcePanel`,
`TestNativeCanonicalArtworkStaysOnFaceWithOutsideCaption`, and
`TestPresentationStackedPersonCaptionsRetainSourceClearance`. Source:
**source_panels.go:69–110; source_panel_density_test.go;
surface_fidelity_test.go:326–399; presentation_caption_test.go:13–123**.

The test-only replay preserves the original input documents, source nodes,
builder state and shared painter constructor counts. Ordered mesh geometry,
material/group/owner identities, every texture pixel hash, source-panel
documents/ranges, errors and budgets compare exactly. All original Go
assertions remain active. Independent caller camera assertions and the
post-call negative-control deformation are outside this call projection.
Initial one-label replay construction was corrected to retain the original
two-label painter and sharing; all original expected results remain identical.
That observer error is excluded from native failures/gains. No normalization,
golden gain or full API parity claim.

**45 native + three harness tests pass**, including all **20 PNG goldens**.
The standalone fixture and input-only external replay both pass. Every one
of **966 e2e classifications** repeats on the unchanged frozen L36 CLI:
**933 SVG / 11 expected errors; fixed 0, regressed 0**. This run adds only
test code; full isometric SVG **870/870** and other tiers remain inherited.

Fresh inventory including concurrent lane observations:
**673 / 472 named / 201 without**. Lane L **seven → four uncovered**,
out of 111 assigned. Remaining: explicit local icon resolution, physical/
structured vector capture, cyclic GIF document admission, and the raster
admission function's three typed-nil Go image calls. No exclusions were added.

Evidence: **lang37/** `panel-sessions`, `panel-{qualified,final}.json`,
`panel-final-native`, `observer-setup.json`, `deltas.json`, native/harness
logs, fresh e2e reports and `verify.py`. Alias:
**isometric-panel-api.json**. Original Go restoration, complete input/output
closure and fixture membership verify. Streaming limits: **1 GiB total /
128 MiB file / 20 GiB free**, finite subprocess deadlines. No commits; other
lanes and `29` preserved.

Cleanup removed **22 setup/duplicate files / 8.16 MiB allocated path
footprint**. Verified **Applesauce 0.5.30 / LZFSE** saved **8.21 MiB** on
this run's retained JSON evidence and fixture. SHA-256, modes, owners and
original xattrs verify; clone backups removed. Exact paths/replacements:
`lang37/{cleanup,compression-evidence}.json`. Rollback:
`applesauce decompress --verify -- <path>`. Local APFS verified;
DiskManagement unavailable. No shared executable/cache, hard link or active
file changed; no bulk moved.

Final allocated/apparent GiB: checkout **22.75/39.71**, `.reference` **11.44/25.28**, `target` **9.85/12.76**; **85.83 GiB free**. Every ceiling passes (`lang37/disk-final.json`; six independent scans, hidden paths/.git and every apparent hard link included). Non-atomic during concurrent work. No builds or parity captures follow the final audit.

### Lane L36: registered surface fonts (2026-10-25)

Read the complete brief through **Update 33**, newest reports/messages and
STATUS; continued Updates 31–32's Lane L split. Fixed the measured configured
font failure before widening the batch: plain text rejected registered bytes,
and rich Markdown could not measure registered primary/mono families.
The shared owned registry lookup now feeds isometric text and scenebuild font
assets; Markdown keeps per-ruler parsed custom families and their distinct
identities. Source: **text.go:103–151; surface_document.go:30–49;
rich_labels.go:222–247; d2scenebuild/build.go:1066–1131;
lib/textmeasure/textmeasure.go:121**. Existing layout arithmetic is unchanged.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / observations | **1/2 → 2/2; 1/6 → 6/6** | **0** |
| Complete scenebuild boards | **1,008 unchanged** | **0** |
| Isometric SVG / source errors | **870 / 138 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

The five failed configured-font observations comprise four errors and one
missing-cache-state consequence; explicit fallback already matched. Original
Go assertions stay active. Full RGBA bytes, layouts, face keys, rich documents
and font bytes, errors, fallback requests and resolver state compare exactly.
The qualified observer adds full fallback-request observation; the verifier
proves its original requests/core expectations equal the negative capture.
The standalone fixture deduplicates ten exact font inputs and binds every
output texture/font byte by length/SHA-256. All new JSON bridges compile only
in tests. No new golden gain or full font/API parity claim.

**43 image + 67 foundation native tests pass**, including all **20 PNG
goldens**; **186 isometric harness tests pass**. After compression the shared
library runner repeats **35 pass / three ordinary external replays ignored**,
and both foundation runners repeat their 67 passes. All **966 e2e
classifications** repeat on the rebuilt frozen CLI. Other full tiers remain
inherited. A concurrent I35 adapter assertion initially blocked the full
native command; the final runner passes after I35's correction. Zero-test
runner/Cargo argument setup attempts are excluded. Scoped rustfmt/Ruff/diff
checks pass; no commits or full concurrent source freeze.

Fresh inventory: **673 added / 453 named / 220 without**,
including F76/I35. **Lane L nine → seven uncovered**, out of 111 assigned.
Next: source-panel density, canonical artwork/stacked-person captions,
explicit icon resolver and physical/structured vector retention. Cyclic Go
pointer graphs and three typed-nil raster calls remain uncovered; no exclusions.

Evidence: **lang36/** `font-{source,qualified}`, `font-negative-ready.json`,
`font-final.json`, `runtime-fix.patch`, `deltas.json`, `verification.json`,
`source-{before,final}.json`, `scope-final.json`, full guard reports/logs and
`verify.py`. Alias: **isometric-font-api.json**. Streaming budgets: **1 GiB
per capture / 128 MiB per file / 20 GiB free**, finite deadlines. Closure
verification covers all original font inputs/outputs, 1,267 pinned SVG input
bindings, exact expected goldens, complete source errors and retired paths.

Cleanup removes **1,748 files / 1,043.84 MiB allocated
path footprint**: obsolete setup/duplicate replay files and successful SVGs
verified byte-identical to retained pinned goldens. Verified **Applesauce
0.5.30 / LZFSE** saves **480.52 MiB / 166 files**, including two idle
foundation test executables and the frozen CLI. Bytes, modes, owners and
original xattrs verify; clone backups are removed. Exact paths/replacements:
`lang36/cleanup.json`, `compression-{foundation,captures}.json`; rollback:
`applesauce decompress --verify -- <path>`. Local APFS verified by mount/df;
no hard links broken, active files rewritten or bulk moved. Savings are
path/operation footprints during concurrent work.

Final allocated/apparent GiB: checkout **22.77/39.72**, `.reference` **11.38/25.20**, `target` **9.92/12.86**; **85.83 GiB free**.
Every ceiling passes (`lang36/disk-final.json`; six complete independent scans,
hidden paths/.git and every apparent hard link included). Measurements are
non-atomic during concurrent work. No builds or parity captures follow this
audit.

### Lane L35: shadow alpha and retained mask/gradient observations (2026-10-25)

Read the full brief, initial Update and Updates through **33**, newest reports
and STATUS; continued the superseding Lane L split in Updates 31–32.
Three original functions add **eight observations**, all already matching.
No renderer algorithm fix was needed. Source: **raster.go:478–492,691–800**,
**vector_surface_svg.go:183–201**, **vector_surface_color.go:26–105**.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / complete observations | **3/3 / 8/8 unchanged** | **0** |
| Prior GIF/output functions / observations | **4/4 / 114/114 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

Every original Go assertion stays active. Full shadow float32-depth and
opacity bytes (20 MiB), camera, bias, work budget, invalid-opacity errors,
mask SVG fragments and COLRv1 paint/writer state compare without tolerance.
Test-only optional-opacity decoding now preserves NaN. The compact native
fixture binds every shadow byte by length/SHA-256; external replay compares
complete buffers. No new golden gain, font-lookup or full API parity claim.
All new bridges and JSON construction compile only in tests.

**39 native tests pass**, including all **20 PNG goldens**; the warm library
runner repeats **33 passed / three ordinary external replays ignored**.
All **176 isometric harness tests**, including the five new failure gates,
pass. All **966 e2e classifications** repeat on the frozen run74 CLI; production
algorithms are unchanged by this lane, and other full tiers are inherited.
Concurrent Lane I34 adapter/FMA failures delayed the native run; the final
complete run passes after Lane I's fixes. Initial observer compilation and
fixture JSON integer/float equality setup failures are excluded.

Fresh inventory: **673 added / 428 named / 245 without**, including the
concurrent I34 qualified observations. **Lane L 12 → nine uncovered**, out of
111 assigned functions. Next: source-panel density, configured/fallback fonts,
explicit icon resolvers and physical/structured vector retention. Cyclic Go
pointer graphs and three typed-nil raster calls remain uncovered; no exclusions.

Evidence: `lang35-{deltas,source,scope-final,verification}.json`,
`lang35-fidelity-qualified-source`, `lang35-fidelity-final{,-native}` and
`lang35-e2e-after`; alias **isometric-fidelity-api.json**. Original source
restoration, every current input/expected output, the fixture's full-byte
bindings and previous raster/animation/output capture closures verify after
cleanup/compression. Streaming **1 GiB total / 128 MiB file / 20 GiB free**
gates and finite deadlines apply. No full concurrent-checkout source freeze.

Cleanup removes **eight obsolete setup/duplicate files / 26.71 MiB allocated
path footprint**. Verified **Applesauce 0.5.30 / LZFSE** saves **55.32 MiB
across 16 files**, preserving SHA-256, modes, ownership and original xattrs;
clone backups removed. Exact paths/reconstruction/rollback:
`lang35-{cleanup,compression}.json`, `applesauce decompress --verify -- <path>`.
Local APFS verified by mount/df; DiskManagement is unavailable. No shared
executable deleted/compressed, no bulk moved. Savings are path/operation
measurements during concurrent work.

Final allocated/apparent GiB: checkout **23.64/39.88**, `.reference` **12.33/25.56**, `target` **9.84/12.67**; **91.55 GiB free**. Every ceiling passes
(`lang35-disk-final.json`; six independent complete scans). A prior scan
caught another lane deleting a temporary directory; that failed scan is
excluded and the complete audit repeats. Hidden paths/.git and every apparent
hard link are included. Measurements are non-atomic during concurrent work.
No builds or parity captures follow the successful final audit. No commits.

### Run 75: board manifest ownership admission (2026-10-25)

Continued Lane F after reading the full brief through Update 33 and the
F74/I33/L34 reports. Fixed the original
`TestValidateBoardOutputManifestRejectsUnsafeOwnershipClaims`: `Manifest::read`
now rejects self-ownership, duplicate portable keys and paths beneath owned
files, including ancestors declared later. It accepts Go's null/empty lists
and enforces the source's 100,000-path limit. Complete diagnostics use Go
quoting. Source: **d2cli/board_output_publish.go:272–331**; existing lowercase
digest comparison follows **499–508**.

| Tier | Before → qualified | Regressions |
|---|---:|---:|
| Original function / complete validation calls | **0/1 → 1/1; 3/16 → 16/16** | **0** |
| SVG/PNG CLI rejection witnesses | **0/6 → 6/6** | **0** |
| Successful uppercase-digest SVG witness | **1/1 unchanged** | **0** |
| Existing publication functions / snapshots | **5/5 / 10/10 unchanged** | **0** |
| Unicode-collision CLI witnesses | **4/4 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

All original Go assertions remain active. Native replay receives input
manifests only; complete errors compare. CLI captures compare complete file
inventories/bytes and error lines at shared paths, require rejected exports to
preserve existing files, and detect leftover staging trees. PNG rows are
rejection contracts, not raster parity claims. **241 native + 9 harness tests
pass**; the normally ignored input-only replay executes through its harness.
All **966 e2e classifications** repeat on the corrected frozen CLI. Other
full tiers are inherited; no new golden gain or full publication/API claim.
Rollback, ancestor replacement and remaining decoder contracts stay open.

Evidence: `run75/{deltas,source,negative-source}.json`, `runtime-fix.patch`,
original `manifest-source`, baseline/qualified API and CLI captures,
`tests-qualified.log` and `qualified-e2e.json`. Aliases:
**board-manifest-api.json**, **board-manifest-cli.json**. Source restoration,
exact run74 negative-runtime reconstruction and input/expected-output closure
verify. Capture guards remain streaming **1 GiB total / 128 MiB file /
20 GiB free**, with finite deadlines. Setup failures and an incorrect
exploratory uppercase-digest expectation are excluded; its corrected positive
witness passes before and after. No commits or full concurrent source freeze.

Fresh inventory: **673 added / 429 named / 244 without**, including concurrent
gains. **Lane F 106 → 105 uncovered**, with CLI **60 → 59**. Next: remaining
original CLI publication/admission tests, prioritizing actual failures.

Cleanup retires **82 files / 60.63 MiB allocated path footprint**: redundant
setup captures and the transient experimental executable. Current requests,
complete expected/actual bytes and negative witnesses remain (`cleanup.json`).
Verified **Applesauce 0.5.30 / LZFSE** saves **34.25 MiB across 64 files**,
preserving SHA-256, modes, owners and original xattrs; clone backups removed.
Exact targets and rollback: `compression.json`,
`applesauce decompress --verify -- <path>`. Local APFS verified; DiskManagement
is unavailable. No bulk moved; these are operation/path measurements, not
net physical-volume attribution during concurrent work.

Final allocated/apparent GiB: checkout **22.47/38.90**, `.reference`
**11.19/24.58**, `target` **9.81/12.67**; **91.17 GiB free**.
Every ceiling passes (`run75/disk-final.json`; six complete independent scans).
Hidden paths/.git and every apparent hard link are included. Measurements
are non-atomic during concurrent work. No builds or parity captures follow
this audit.

### Lane L34: GIF output contracts and mask clone state (2026-10-25)

Read the complete brief through **Update 33** and continued the superseding
Lane L file split in Updates 31–32. Four original functions add **114 native
observations**. `TestNativeGIFLoopCopiesAnimatedMasks` exposed absent child
lists in the clone where Go allocates empty lists, at both the root and mask
root. `gif_loop.rs` now follows **gif_loop.go:49**, preserving the caller's
absent lists. The other three functions already matched. Source mapping:
**gif.go:105–189**, **gif_loop.go:39–68**, **framing.go:66–131**.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / complete observations | **3/4 → 4/4; 113/114 → 114/114** | **0** |
| Existing GIF calls / composited frames | **17 / 592 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All original Go assertions remain active. The entire 262,144-byte palette
lookup, indexed images, delta rectangles/pixels/palettes, delays, writer
bytes/errors/budget, mask documents/ownership and shadow extents compare.
Transport and JSON construction compile only in tests. Cyclic Go pointer
graphs, arbitrary writer callbacks and encoded-GIF byte identity remain
outside this finite replay. No new golden gain or full API parity claimed.

**37 native + 50 harness tests pass**, including all **20 PNG goldens**;
the same 31-test warm library runner repeats after compression. Three unrelated
external replays remain ordinarily ignored. All **966 e2e classifications**
repeat on the rebuilt CLI. Other full tiers are inherited. New source and
comparator formatting checks pass; the full gif.rs check still flags its
inherited long palette_lookup signature, left untouched. Setup failures
(missing runner/Pillow, the initial default-cache build and a corrected vector
decoder) are excluded from parity evidence. No commits or full concurrent
source freeze.

Fresh inventory: **673 added / 405 named / 268 without**, including concurrent
gains. **Lane L 16 → 12 uncovered**, out of 111 assigned functions. The three
Go typed-nil raster calls remain unsupported; no applicability exclusions.
Next prefer shadow alpha/admission and retained-vector/font/resolver contracts.

Evidence: `lang34-{deltas,source,verification,retention-verification}.json`,
`lang34-output-source`, negative/final replay, `lang34-e2e-after`,
`lang34-gif-final.json`; alias **isometric-output-api.json**. Every retained
request/expected output, negative response and retirement reconstruction
verifies, including 1,267 pinned SVG inputs and previous raster captures.
Capture limits: **1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.

Cleanup removes **13 duplicate files / 1.23 MiB allocated path footprint**;
verified **Applesauce 0.5.30 / LZFSE** saves **5.16 MiB across 17 files**.
SHA-256, modes, owners and original xattrs verify; clone backups are removed.
Exact paths/reconstruction/rollback: `lang34-{cleanup,compression}.json`.
No shared executables deleted or compressed; no bulk moved. Default-cache
dependency paths were replaced in place by canonical-cache builds, with no
duplicate bodies remaining. These are path/operation measurements.

Final allocated/apparent GiB: checkout **22.27/38.54**, `.reference` **11.09/24.27**, `target` **9.72/12.61**; **92.70 GiB free**. Every ceiling passes (`lang34-disk-final.json`; six complete independent scans exit zero). Hidden paths/.git and every apparent hard link are included. Measurements are non-atomic during concurrent lane work. No builds or parity captures follow this audit.

### Run 74: Unicode board-output collision admission (2026-10-25)

Read the full brief through **Update 33**, newest reports/messages and STATUS;
incorporated arriving I32/L33/L34. Continued Lane F at master **6a7b48378**.
Fixed four demonstrated CLI failures: SVG and PNG accepted colliding
`é`/`e\u0301` and `Straße`/`STRASSE` board names, while Go rejected them
before publishing files. Source: **d2cli/board_output.go:281–283**.
The shared key now cleans paths lexically, performs full Unicode folding,
and applies NFC with Go's stream-safe nonstarter policy. Collision errors
use Go `%q` spelling. Stale aliases additionally require actual filesystem
identity (**board_output_publish.go:474–482,815–830**).

| Tier | Before → final | Regressions |
|---|---:|---:|
| SVG/PNG collision admission witnesses | **0/4 → 4/4** | **0** |
| Original publication functions / complete snapshots | **5/5 / 10/10 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

All **966 e2e classifications** repeat. The five original Go functions retain
all assertions; complete file inventories, bytes and ownership hashes compare.
The observer retains both physical and manifest spellings, reading declared
paths through the filesystem; no path normalization or digest rewriting in
the comparator. Go publication captures explicitly use the release version
stamp **v0.9.0**. No new upstream golden gain or complete publication/platform
parity claimed; other full tiers are inherited.

**240 native CLI + 5 harness tests pass**, including 23 pinned-Go Unicode/path
records. Go 1.27 selects **Unicode 17**, so native NFC uses the unmodified
vendored **unicode-normalization 0.1.25** source; generated folding/nonstarter
tables come from pinned **x/text v0.41.0**. Original licenses/notices remain.
Seven focused native tests and all four CLI witnesses repeat after compression.
Ruff, Go formatting, locked/offline metadata and scoped diff checks pass.

Evidence: `.reference/codex-runs/run74/{deltas,source,vendor-source}.json`,
negative/final scoreboards and original capture inventories. Aliases:
**board-output-api.json**, **board-output-collisions.json**. The verifier
checks source restoration, exact snapshot/manifest bytes, scoped source,
vendor hashes and an otherwise unchanged Cargo lock. Streaming **1 GiB total /
128 MiB file / 20 GiB free** gates and finite deadlines apply. No full
concurrent-checkout freeze. Early observer/setup attempts, live-clock PDF
exploration and superseded dependency/cache build attempts are excluded.

Fresh inventory: **673 added / 405 named / 268 without**, including concurrent
Lane I gains. **Lane F 111 → 106 uncovered**: CLI 60, SVG four, layouts 31,
JS CI two and PNG/report nine. Next prefer the remaining original CLI
publication/admission contracts, selecting demonstrated failures.

Cleanup retires **121 paths / 5.34 MiB allocated path footprint**, including
resolved setup captures and duplicate experimental Unicode source/archives
(`run74/cleanup.json`). The thin shared Cargo cache view remains because
another lane adopted it; existing dependency sources are symlinks, not copies.
Verified **Applesauce 0.5.30 / LZFSE** saves **215.30 MiB across 29 retained
files**, preserving SHA-256, modes, owners and original xattrs; clone backups
are removed. Targets/rollback: `run74/compression.json`,
`applesauce decompress --verify -- <exact-path>`. No bulk moved.
These are path/operation figures, not concurrent net volume attribution.

Final allocated/apparent GiB: checkout **22.27/38.54**, `.reference` **11.09/24.28**, `target` **9.72/12.61**; **92.70 GiB free**. Every ceiling
passes (`run74/disk-at-stop.json`; six independent complete scans). Hidden
paths/.git and every apparent hard link are included. Measurements are
non-atomic during concurrent work. No builds or parity captures follow this
audit. No commits; other lanes and `29` preserved.


### Run 73: registered custom families in raw SVG (2026-10-25)

Read the complete brief through **Update 33**, newest reports/messages and
STATUS. Continued Lane F's remaining SVG queue. Original
`TestRenderAcceptsRegisteredCustomFontFamily` failed because validation and
embedding recognized only the three bundled family names. Added the shared
named-face registry, SourceSansPro fallback for omitted styles, and lookup
from both validation and embedding. Source: **d2fonts_common.go:38–61,116–209**;
**d2target/render_validation.go:264–277**. Exposed the existing direct
`svg_theme_css` implementation (**d2svg.go:3296–3318**) for its original tests.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original registered-font / ThemeCSS calls | **5/6 → 6/6** | **0** |
| Prior raw SVG admission / structural / label calls | **60/31/10 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Raw SVG goldens | **939/952 → unchanged** | **0** |

All six complete responses compare, including the custom-family SVG and five
complete stylesheet errors; every original Go assertion remains active.
The negative adapter uses the pre-existing renderer without a named-family
registration API. All 966 e2e classifications remain unchanged. The raw tier
keeps 11 stale Go matches and two Go decode errors non-passing; no new golden
or complete custom-font/API parity claimed. Other full tiers are inherited.
**281 native + 10 harness tests pass**; all five compressed warm runners repeat.
Supplied-face/re-registration tests, release WASM target compilation, scoped
rustfmt/Ruff and diff checks pass. No commits; other lanes and `29` preserved.

Evidence: `.reference/codex-runs/run73/{deltas,source,verification}.json`,
original `font-capture`, exact native fixture and complete negative/final
responses. Alias: `svg-font-api.json`. Final guards have streaming **1 GiB
total / 128 MiB file / 20 GiB free** limits and finite deadlines. Initial
exploratory guards and unsuccessful external-monitor setup are excluded;
only the bounded final guards count. The verifier checks original source
restoration, 1,267 inherited input bindings, all 328 e2e cases and 952 goldens.
No full concurrent source freeze.

Inventory: **673 added / 349 named / 324 without**, including concurrent gains.
**Lane F 113 → 111 uncovered**: CLI 65, SVG four, layouts 31, JS CI two and
PNG/report nine. Next prefer original CLI contracts or the remaining direct
SVG validation tests, selecting demonstrated failures. Browser/Node package
execution remains unqualified.

Cleanup retires **214 paths / 296.42 MiB allocated path footprint**, preserving
inputs, expected outputs and negative witnesses (`run73/cleanup.json`).
Verified Applesauce **0.5.30 / LZFSE** saves **125.13 MiB across 10 files**;
hashes, modes, ownership and original xattrs verify; clone backups are removed.
Exact targets/rollback: `run73/compression.json`. No bulk moved. Figures are
path/operation measurements during concurrent work.

Final allocated/apparent GiB: checkout **22.43/38.36**, `.reference` **11.33/24.35**, `target` **9.64/12.36**; **92.70 GiB free**. Every ceiling passes (`run73/disk-final.json`; six independent complete du scans exit zero). Hidden paths/.git and every apparent hard link are included. Measurements are non-atomic during concurrent lanes. No builds or parity captures follow this audit.


### Lane I33: solid-contact routing preserves cancellation (2026-10-25)

Read the full brief through **Update 33** and incorporated F74/L34. Continued
Lane I's superseding file split. Fixed original
`TestQueueContactRoutesPreserveSourceVerticesAndBounds`: Rust's final
solid-contact stage dropped the caller's context and returned a route after
cancellation. It now receives that context and checks it before each route,
as **solid_contacts.go:79–137** does. The background-context convenience
wrapper remains available; contact geometry is unchanged.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / complete observed calls | **19/20 → 20/20; 222/223 → 223/223** | **0** |
| Native isometric SVG / source errors | **870 / 138 unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 unchanged** | **0** |

The batch also compares exact solid contacts/support offsets, projected
caption obstacles/work, casing floors, sidewall vertices/normals/materials,
source paint metadata and classic palette nodes. All original Go assertions
stay active. Native replay receives only requests. Shared projection-material
identity, alpha/texture presence and nonfinite point classifications survive
transport; no geometry tolerance or normalization. Unobserved callers, raster
assertions, Go allocation/map layout and pointer addresses are outside this
projection. No new golden gain or full API parity claim.

**40 native + 4 harness tests pass**, including all **20 PNG goldens**.
Three unrelated external replays remain ordinarily ignored. The full fresh
isometric guard reproduces all 870 pinned SVGs and 138 errors; every one of
its 1,008 classifications is unchanged. E2e repeats all 966 classifications
on the frozen I25 CLI; the final bounded run also contains its temporary
outputs. Other full tiers are inherited. All captures use streaming
**1 GiB total / 128 MiB file / 20 GiB free** guards and finite deadlines.
Scoped rustfmt/Ruff/diff checks pass. Initial adapter compile/count setup and
interrupted queued builds are excluded. Qualified builds use the absolute
shared Cargo cache; no new executable checkpoint copy or full concurrent
source freeze was created.

Fresh inventory after F74/L34: **673 added / 405 named / 268 without**.
**Lane I: 155 → 135 image functions**, plus the one scene URL/Userinfo
handoff: **156 → 136 total**. No applicability exclusions. Next: remaining
route-caption masks and native solid/structured contracts, prioritizing any
observed failure before widening.

Evidence: `.reference/codex-runs/iso33/`: `contact-qualified`, complete
negative/final native replays, `runtime-fix.patch`, `verification.json`,
`scope-sealing.json` and `verify.py`. Alias: **isometric-contact-api.json**.
The verifier checks exact negative-source reconstruction, source restoration,
original request/expected-output closure, 1,267 retained pinned SVG inputs,
complete error witnesses and every compared SVG hash. Supervisor paths and
rebuild commands are in **report-iso33.md**. No commits.

Cleanup removes **1,772 files / 967.80 MiB allocated path footprint**, including
redundant successful SVG copies whose exact pinned goldens remain, duplicate
setup captures and temporary source backups. The negative witness and reversible
source patch remain (`iso33/cleanup.json`). Verified **Applesauce 0.5.30 / LZFSE**
saves **2.84 MiB / 35 retained files**, preserving hashes, modes, owners and
original xattrs; clone backups are removed. Rollback:
`applesauce decompress --verify -- <exact-path>` from `iso33/compression.json`.
No shared executable was deleted/compressed; no bulk moved. Figures describe
path/operation footprints, not concurrent net-volume attribution.

Final allocated/apparent GiB: checkout **22.27/38.54**, `.reference` **11.09/24.27**, `target` **9.72/12.61**; **92.70 GiB free**. Every ceiling passes (`iso33/disk-final.json`; six complete independent
scans). Hidden paths/.git and every apparent hard link are included.
Measurements are non-atomic during concurrent work. No builds or parity captures
follow this audit.

### Lane I32: route-caption and physical-outline observations (2026-10-25)

Continued the full brief through **Update 33**, using the superseding Lane I
file split; incorporated F73/L33. Captured **20 previously uncovered original
functions / 74 calls**, all matching on their first native replay. **No renderer
fix or production algorithm change.** The only existing module edit adds a
`cfg(test)` replay child.

The batch compares authored caption anchors/metrics, complete obstacle state,
resource-budget fallbacks, oriented overlap, clipped dash paths, rounded routes,
and complete physical-ink segments/support facets/depth/clipped faces. Original
Go assertions stay active. Source: **route_labels.go:58–316**,
**route_caption_style.go:33–194**, **geometry_routes.go:59–105** and
**native_edges.go:68–84,102–373,594–705**. Exact consecutive-rectangle encoding
preserves all 19,000 budget-witness obstacles; nonfinite dimensions survive
transport. No numeric tolerances or geometry normalization. Go map layout,
pointer addresses, allocations and unobserved painter/raster behavior remain
outside this replay. No new golden gain or complete API parity claim.

**38 native + 4 harness tests pass**, including all **20 PNG goldens**. The
ordinary e2e guard repeats **933 SVG matches + 11 expected errors**, all **966
classifications unchanged**, zero regressions against the master baseline.
E2e uses the frozen I25 CLI; production code is unchanged by this batch. Full
870-board SVG and other full tiers remain inherited. Three unrelated external
replays stay ignored in this ordinary unit run. Scoped Ruff/rustfmt/diff checks
pass. Initial transport compilation, one provisional operation count, and a
verifier schema error were corrected; none is counted as parity evidence.

Fresh inventory: **673 added / 376 named / 297 without**, including concurrent
Lane L gains. **Lane I: 175 → 155 image functions**, plus one scene URL/Userinfo
handoff, **156 total**. No applicability exclusions. Next prefer remaining
caption-mask, casing and solid-contact tests, selecting actual failures first.

Evidence: `.reference/codex-runs/iso32-caption-source`, full final native
replay, `iso32-{source,scope-final,verification}.json` and `iso32-verify.py`;
alias **isometric-caption-api.json**. Original input/expected-output closure,
exact permanent fixture, restored Go tests and input-only replay verify.
Limits remain **1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.
No full concurrent-checkout source freeze or commits. Commands and supervisor
paths are in **report-iso32.md**.

Cleanup removes **98 superseded files / 7.55 MiB allocated path footprint**,
after matching all successful observations to the retained 20-function corpus.
Verified **Applesauce 0.5.30 / LZFSE** saves **3.90 MiB across 27 retained
capture/replay/log files**, preserving SHA-256, modes, ownership and original
xattrs; clone backups are removed. Exact paths/rollback:
`iso32-{cleanup,compression}.json`, `applesauce decompress --verify -- <path>`.
Shared executables are untouched by compression. No bulk moved.

Final allocated/apparent GiB: checkout **22.10/38.20**, `.reference` **11.03/24.21**, `target` **9.60/12.34**; **93.84 GiB free**. Every ceiling passes (`iso32-disk-final.json`; six independent complete scans). Hidden paths/.git and every apparent hard-linked path are included. Measurements are non-atomic during concurrent work. No builds or parity captures follow this final audit.

### Lane L33: animation and shadow raster-call observations (2026-10-25)

Read the full brief through **Update 33**, newest reports and STATUS; continued
Lane L's superseding Updates 31–32 scope, incorporating I31/F73. Seven original
master-added functions now have matching native raster observations:
**250/250 calls**, including authored-node/packet loops, animated source panels,
tiny-node/traffic framing, fixed-camera motion, surface decals and solid-ground
shadow framing. Every Go assertion remains active. **No renderer algorithm
change was needed**, and no new golden gain or complete animation API parity
is claimed. Caller geometry, raw texture bytes and explicit view/shadow/ground
cameras are replay inputs; caller motion, loop adaptation and camera construction
remain Go-only assertions in this tier. Source: **raster.go:165–260**.

The test-only transport admits raw texture-file inputs and the explicit ground
camera; all JSON/transport construction compiles only under `cfg(test)`.
Repeated texture bytes initially exceeded the 128 MiB file budget; that attempt
was stopped and excluded. The qualified capture retains exact texture bytes
once: **82 MiB total / 49 MiB maximum file**, under the unchanged streaming
**1 GiB total / 128 MiB file / 20 GiB free** limits and finite deadlines.
An outdated warm runner's missing transport support is also excluded setup
failure, not a renderer baseline.

**Zero measured regressions**: the prior storage suite remains **9 matching
functions / 15,150 operations**, with its three typed-nil calls still explicitly
unsupported; the prior raster suite remains **7 functions / 28 calls**.
Ordinary e2e repeats all **966 classifications: 933 SVG + 11 expected errors**,
using the frozen L27 CLI. **41 native + 63 raster-harness + 38 raster-gate tests
pass**, including all **20 PNG goldens**. The new external replay executes
explicitly; three external-capture tests remain ignored in ordinary unit runs.
Full SVG, language and other tiers are inherited; native production code was
not changed in this run.

Fresh inventory: **673 added / 356 named / 317 without**, including concurrent
lane gains. **Lane L: 23 → 16 uncovered**, out of 111 assigned functions; no
applicability exclusions. Remaining: GIF clone/mask admission, palette/writer,
source-panel density, shadow-alpha/non-caster, presentation and retained-vector/
font/resolver contracts, plus the typed-nil-only raster residual. Evidence:
`lang33-{deltas,source,scope-final,verification}.json`, original
`lang33-animation-qualified`, complete final native responses and cleanup ledger.
New alias: **isometric-animation-raster-api.json**. Commands and exact supervisor
paths are in **report-lang33.md**. No commits or full concurrent source freeze.

Cleanup retires **169 files / 723.84 MiB allocated path
footprint**, plus 3 empty directories: the failed bulk attempt, redundant
successful replay bytes and the idle checkpoint runner. Exact hashes, replacements
and rebuild commands: `lang33-cleanup.json`. Original inputs, expected outputs,
failure logs and historical runner hashes remain; no bulk moved.

Verified **Applesauce 0.5.30 / LZFSE** saves **167.22 MiB across
168 retained files**, preserving SHA-256, modes, owners and original xattrs;
clone backups are removed. Exact targets and rollback commands:
`lang33-compression.json`, `applesauce decompress --verify -- <exact-path>`.
Local APFS verified. These are path/operation footprints, not net-volume attribution.
Another lane rebuilt shared warm executables: original **41-test** bindings remain
explicit, and current compressed warm executables pass **42 tests**. The initial
post-compression hash check detected that replacement and was corrected; it is
not counted as a passing repeat of the historical executable.

Final allocated/apparent GiB: checkout **21.99/38.08**, `.reference`
**10.92/24.09**, `target` **9.60/12.34**; **93.84 GiB free**.
Every ceiling passes (`lang33-disk-final.json`; six complete independent scans
exit zero). Hidden paths/.git and every apparent hard link are included.
Measurements are non-atomic during concurrent lane work. No further builds or
parity captures follow this audit.

### Lane L32: raw raster storage, precision and owned snapshots (2026-10-25)

Continued the complete brief through **Update 33**, using Lane L's superseding
Updates 31–32 scope; incorporated I30/F72. Captured ten original raster
functions, preserving every Go assertion. Added native strided RGBA/NRGBA/Alpha
storage, overflow-safe row admission and 16-bit NRGBA sampling, plus owned
snapshots that preserve color models and internal material/texture/group/owner
sharing. Public deferred renders use the snapshot. Source:
**raster.go:158–171,307–393,557–569,1069–1107** and Go `color.NRGBA.RGBA`.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Functions with matching observed calls | **7 → 9**, out of ten | **0** |
| Observations | **15,137 → 15,150 matches**, out of 15,153 | **0** |
| Complete isometric SVG / expected errors | **870 / 138 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All 13 previously unsupported native operations now match. Three Go typed-nil
image calls remain explicitly unsupported, keeping their admission function
uncovered. No applicability exclusion, golden-board gain or full API parity
claim. The capture compares complete raw pixels with strict nonempty file
inventories, byte hashes and storage checks, full snapshots/samples/errors,
15,113 scanline spans and compositor values. Go cache layouts, forced-strip
internals and unobserved capture-camera assertions remain Go-only assertions.
The original integer observer round-trip and the first fixture's JSON numeric
representation check were corrected; failed attempts never count as passes.

**38 native + 45 harness + 38 raster-gate tests pass**, including all **20 PNG
goldens**. Seven warm executables repeat the native suite after compression.
The previous raster guard remains **7 functions / 28 calls**. Both ordinary
e2e runs keep all **966 classifications**; all **1,008 isometric classifications**
repeat with the rebuilt probe. E2e uses the frozen L27 CLI. Full L30 surface,
L31 content and other full tiers remain inherited; original retained byte
bindings are verified. The new external replay is explicitly executed; two
other external-capture tests stay ignored in ordinary unit runs. The permanent
fixture retains two complete functions / 12 precision-compositor observations.
Replay and diagnostic JSON construction compile only under `cfg(test)`.

Fresh inventory: **673 added / 345 named / 328 without** at measurement,
including concurrent lane gains. **Lane L: 32 → 23 uncovered**, out of 111
assigned added functions. Next: remaining animation/GIF, surface/vector and
presentation tests; capture actual failures before expanding. Evidence:
`.reference/codex-runs/lang32-{deltas,source,scope-final,verification}.json`,
qualified original capture, complete baseline/final observations and explicit
retirement ledger; alias **isometric-raster-storage-api.json**. Reproduction
commands and exact supervisor paths are in **report-lang32.md**. Limits:
streaming **1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.
No full concurrent-checkout source freeze or commits.

Cleanup retires **10,566 files / 3,325.52 MiB allocated path footprint**,
plus 2,167 empty directories: verified duplicate captures, temporary
runner copies/setup files and idle Go action-cache entries. Original inputs,
expected outputs, negative witnesses and historical hashes/rebuild records remain
(`lang32-cleanup.json`). Cache activity was checked with `lsof +D` before each
batch; newer/concurrently repopulated cache entries remain. No bulk moved.

Verified **Applesauce 0.5.30 / LZFSE** saves **185.69 MiB across
45 retained files**, preserving SHA-256, modes, ownership and original xattrs;
clone backups are removed. Exact paths and rollback commands:
`lang32{,-tail}-compression.json`, `applesauce decompress --verify -- <exact-path>`.
Local APFS verified. Savings are path/operation measurements, not concurrent
net physical-volume attribution. Historical runner copies are explicitly retired.

Final allocated/apparent GiB: checkout **22.07/38.08**, `.reference`
**10.81/23.83**, `target` **9.80/12.60**; **93.46 GiB free**. Every
ceiling passes (`lang32-disk-final.json`; six complete independent scans exit
zero). Hidden paths/.git and every apparent hard link are included. Measurements
are non-atomic during concurrent lane work. No builds or parity captures follow
this final audit.

### Lane I31: outline and solid-ink cancellation (2026-10-25)

Re-read the entire brief through **Update 33** and newest reports, including
arriving F72/L32. Fixed two demonstrated original-test failures in Lane I's
Update 31 scope. `TestClassicInkCancellationAndWorkLimit` returned nine segments
for a cancelled cube without an error: extraction now receives the caller's
context and polls every 1,024 triangle, edge and support-adjacency visits
(**native_edges.go:102–126,270–279,305–312,471**).
`TestNativeSolidInkFitCancellation` omitted cancellation from its fallback:
texture fitting now checks every 64 rows and records the context error
(**native_solids.go:115–118,133–139**). Source input-limit precedence and
nil/empty behavior remain intact. No geometry or FMA change.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Outline admission calls | **1/2 → 2/2** | **0** |
| Ink-fit call | **0/1 → 1/1** | **0** |
| Full native isometric SVG / expected errors | **870 / 138 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

Both original Go functions keep every assertion. Complete admission errors,
segment counts and fit factors compare on input-only native replay. Consumed
triangle fields and raw texture bytes survive transport; all 100,001 zero
input triangles are checked before exact count encoding. Successful segment/
support geometry and asynchronous cancellation timing are outside this small
admission projection. No new golden gain or complete API parity claimed.
The two real failures took priority over a wider confirmation batch.

**37 native + 3 harness tests pass**, including all **20 PNG goldens**.
The rebuilt isometric probe repeats every **1,008 classification**; frozen
I25/F69 ordinary CLI repeats every **966 classification**. Three unrelated
external-capture tests remain ignored in ordinary unit runs; their full
qualification and other complete tiers are inherited. Scoped Ruff, rustfmt
and diff checks pass. Native replay/JSON code compiles only in tests.

Fresh inventory: **673 added / 347 named / 326 without**, including concurrent
Lane L gains. Lane I image **177 → 175 uncovered**, plus the scene URL/Userinfo
handoff: **176 total**. No applicability exclusions. Next prefer remaining
route-caption admission and native-ink contracts, selecting actual failures.

Evidence: `.reference/codex-runs/iso31-{deltas,source,scope-final,runner-bindings}.json`,
original `iso31-ink-capture-combined`, both staged negative witnesses and
final results; new alias `isometric-ink-api.json`. `iso31-verify.py` checks
original assertions, exact fixture, source restoration, negative witnesses,
regression classifications and retained input/expected-output closure,
including **1,267 pinned SVG inputs**. Outline and fitting negatives were
observed before their respective fixes; no synthetic combined pre-fix run.
Full commands and file list: `report-iso31.md`. Captures enforce streaming
**1 GiB total / 128 MiB file / 20 GiB free** and finite deadlines. A mistyped
frozen CLI path, supplemental test setup errors and executable bits were
corrected; failed attempts never count as passes. No commits initiated.

Cleanup retires **1,741 files / 967.15 MiB allocated path footprint**: exact successful SVG copies and one duplicate original input/expectation, with pinned golden or qualified-capture replacements verified first (`iso31-cleanup.json`). Both staged negative witnesses, original combined inputs/expectations, full error witnesses, hashes and rebuild records remain. No bulk moved.

Verified **Applesauce 0.5.30 / LZFSE** saves **2.02 MiB across 11 retained files**, preserving SHA-256, modes, ownership and original xattrs; clone backups are removed. Exact targets and rollback: `iso31-compression.json`, `applesauce decompress --verify -- <exact-path>`. Six shared executables were skipped while Cargo was active; all six warm executables subsequently repeat **37 tests**, and the complete three-call replay repeats. Local APFS verified by mount/df. Savings describe file operations during concurrent work, not net physical-volume attribution. Historical shared-runner hashes stay explicit; no additional checkpoint executable retained.

Final allocated/apparent GiB: checkout **22.03/37.90**, `.reference` **10.91/23.93**, `target` **9.66/12.32**; **93.08 GiB free**. Every ceiling passes (`iso31-disk-final.json`; six independent complete du scans exit zero). Hidden paths/.git and every apparent hard-linked path are included. Measurements are non-atomic during concurrent lanes. No builds or parity captures follow this audit.

### Lane I30: cached endpoint-visibility cancellation (2026-10-25)

Continued the full brief through **Update 33**. Original
`TestArrowVisibilityBoundsCachesWorkAndCancellation` exposed a missed context
check: a cancelled cached query moved its marker and debited more work.
`arrow_visibility.rs` now checks the context before every work debit and
passes it to the marker/cap helper builders. Source:
**arrow_visibility.go:35–47,75,114**. No geometry/FMA change.

Eight original Go functions retain every assertion. Input-only native replay
improves **7/8 → 8/8 functions; 50/51 → 51/51 queries**, **zero regressions**.
Complete routes/source paths, errors, work and cache counts compare exactly;
receiver polygons, marker vertices and consumed triangle/material fields are
query inputs. Go allocation identities and unobserved overlap/pipeline details
remain outside those observations. No additional golden-board gain or complete
API parity claimed. The demonstrated failure took priority over widening.

**31 native + 28 harness tests pass**, including all **20 PNG goldens**.
Header/bridge **43/43** and lane **28/28** calls remain unchanged. Two unrelated
external-capture tests stay ignored in the ordinary unit run; their previous
full surface/content qualification is inherited. Ordinary e2e repeats all
**966 classifications: 933 SVG + 11 expected errors**, using the frozen I25/F69
CLI. The rebuilt isometric probe repeats **870 SVG + 138 expected errors**
across all **1,008 classifications**. After concurrent Lane F additions, all
six compressed warm executables pass **34 tests**, with three external-capture
tests ignored; historical runner hashes remain explicit. Final cleanup/disk evidence is below.

Fresh inventory: **673 added / 332 named observations / 341 without**.
Lane I image **185 → 177 uncovered**, plus the scene URL/Userinfo handoff:
**178 total**. No applicability exclusions. Next prefer remaining route-caption
and native-ink admission contracts, selecting demonstrated failures.

Evidence: `iso30-{deltas,source,scope-final,runner-bindings}.json`, original
`iso30-arrow-capture-final`, negative/final replay and `iso30-verify.py`.
The compact native fixture retains all four cache/budget calls. A separate
native test checks cancellation after a debit, precedence over exhaustion and
first-error preservation. Capture limits remain **1 GiB total / 128 MiB file /
20 GiB free**, streaming guards and finite deadlines. Failed edge-count and
unexported-point setup projections never count as passing. Concurrent Lane F
adapter compile errors were fixed by their owner before the successful probe
rebuild; Lane I did not edit those files. No commits initiated.

Cleanup retires **1001.82 MiB allocated path footprint** in 1759 files; original input/expected-output closure and negative witnesses remain (`iso30-{cleanup,retirement}.json`). Verified Applesauce **0.5.30 / LZFSE** saves **70.88 MiB** across 25 operations, preserving hashes, modes, ownership and original xattrs; clone backups are removed. Full file list, rebuild/rollback records and limitations: `report-iso30.md`. No bulk moved.

Final allocated/apparent GiB: checkout **24.82/40.63**, `.reference` **13.52/26.36**, `target` **9.84/12.62**; **90.03 GiB free**. Every ceiling passes (`iso30-disk-final.json`; six independent complete du scans exit zero). Hidden paths/.git and every apparent hard-linked path are included. Measurements are non-atomic during concurrent lanes. No builds or parity captures follow this audit.

### Lane L31: retained-surface cancellation and 20-function content batch (2026-10-25)

Re-read the full brief through **Update 33** and newest reports. Updates 31–32
supersede the old imgbundler assignment. Captured 20 original isometric-image
functions, preserving every Go assertion. Fixed
`TestNativeVectorCaptureIsScopedAndCancelable`: the surface SVG writer ignored
the caller's cancellation. `native_surface_svg_with_context` now checks the
export context at each writer admission and propagates it through recursive
coverage surfaces; the renderer supplies that context. Source:
**vector_surface_svg.go:38–110,111–125**. No geometry/raster algorithm change.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions | **19/20 → 20/20** | **0** |
| Existing observations | **77/78 → 78/78** | **0** |
| Expanded direct mutation observations | **5/5 final** | **0** |
| Isometric SVG / expected errors | **870 / 138 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All **83 observations**, **1,008 isometric classifications** and **966 e2e
classifications** qualify. The isometric probe is rebuilt; ordinary e2e uses
the frozen hash-verified L27 CLI. Previous surface contracts remain
**24/24 / 2,910 operations**. **29 native + 19 harness + 38 raster-gate tests
pass**, including all **20 PNG goldens**. Both ignored external-capture tests
are explicitly exercised. Other full tiers remain inherited; no new golden
gain or complete API parity is claimed.

Comparison retains complete ordered geometry/materials/texture pixels,
documents, SVG fragments and errors. Direct aperture/opacity edits execute
the native helpers. Request transport preserves Go's `-0` float input;
14 small complete functions / 67 operations form the native fixture. Go
registry isolation/allocation internals, private reimport tolerances and
unobserved packet details remain original Go assertions. Replay/JSON code is
test-only. Namespace/type/borrow setup errors were corrected and never count
as passes. Shared Cargo replacements retain explicit historical runner hashes.

Fresh inventory: **673 added / 324 named observations / 349 without**.
**Lane L: 52 → 32 uncovered**, out of 111 assigned added functions; no
applicability exclusions. Next prefer raster admission/AA and remaining
surface/animation tests, capturing failures before expanding.
Evidence: `.reference/codex-runs/lang31-{deltas,source,verification}.json`,
qualified original capture, complete negative witness and sealed replay;
alias `isometric-content-api.json`. Full commands/file list:
`.reference/codex-runs/report-lang31.md`. Limits: **1 GiB total / 128 MiB file /
20 GiB free**, streaming guards and finite deadlines. Input closure reuses
the verified 1,267 pinned bindings in `iso29-svg-inputs.json`.

Cleanup retires **1,884 files / 2,322.09 MiB allocated path footprint**, plus
2,174 empty directories: exact golden copies, redundant successful replays
and temporary generators. Original inputs/expectations, negative witnesses,
historical hashes and reproduction records remain (`lang31-cleanup.json`).
No bulk moved. Verified Applesauce **0.5.30 / LZFSE** retains **256.02 MiB**
savings across 27 files; 34 operations saved 404.20 MiB before concurrent
retirement of four old executables and replacement of the shared unit runner. Hashes/modes/owners/xattrs verify and backups are removed;
exact targets/rollback are in `lang31-*-compression.json`. One tiny JSON stays
uncompressed at the tool's threshold. These are path/operation measurements,
not net concurrent-volume savings.

Final allocated/apparent GiB: checkout **23.19/39.34**, `.reference` **11.85/24.68**, `target` **9.87/13.01**; **92.32 GiB free**. Every ceiling passes (`lang31-disk-final.json`; six independent scans exit zero). Hidden paths/.git and every apparent hard link are included. Measurements are non-atomic during concurrent lane work. No builds or captures follow this audit.

### Run 72: direct isometric SVG API and WASM empty-board audit (2026-10-25)

Completed brief Update 33's first investigation through the **actual pinned
Go WASM `Render` entry**, using all three `sanity/empty` target inputs. Full
native responses match **3/3**; Go WASM reproduces **0/3** old SVG goldens.
WASM renders the selected single board, whereas the e2e adapter traverses
folder-only boards and wraps an empty frame list. These remain non-passing
golden rows; no renderer change or package rebuild can make the differing
entry-point contracts the same. Browser/Node package execution stays queued.

Then captured four original `d2svg/isometric_test.go` functions, retaining
every assertion. Added `d2_rs_isometric::svg::{render,render_isometric,
render_multiboard}` above the flat renderer to avoid a dependency cycle.
The native facade preserves document options, native dimensions/context,
asset admission, empty override pointers and static board ordering. WASM
now calls it too. Source: **d2svg.go:2866–2909,3575–3628** and
**isometric.go:21–96**; the existing document wrapper is reused.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Direct SVG calls, four original functions | **3/13 → 13/13** | **0** |
| Prior WASM isometric / admission calls | **12/12 / 5/5 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| WASM target golden guard | **936/952 → unchanged** | **0** |

The negative SVG baseline is the pre-existing flat target primitive,
**not a claimed prior RenderIsometric implementation**. All 13 full responses
compare, including five SVG documents in four successful calls, complete
errors and ordered board arrays. Inf scale inputs preserve their IEEE bits.
All **966 e2e / 952 WASM classifications** repeat; the latter keeps the three
entry-point differences, 11 stale Go matches and two Go decode errors visible.
No new golden-board gain or full API parity claimed. Other full tiers are
inherited. **103 native + 53 harness tests pass**, including the native tests
repeated from compressed warm executables. WASM release target compilation,
scoped formatting/Ruff and diff checks pass. No package published or commits
initiated; other-lane changes and `29` preserved.

Evidence: `.reference/codex-runs/run72-{deltas,source,verification}.json`,
original `run72-svg-isometric` and `run72-wasm-empty` captures, negative/final
native reports and fresh e2e/WASM guards. `run72-verify.py` verifies original
assertions, fixture bytes, input/expected-output closure, scoped sources,
runner hashes and Go restoration. Captures have streaming **1 GiB total /
128 MiB file / 20 GiB free** limits and finite deadlines. Initial visibility,
xattr-tool and mutable-Cargo-alias verification attempts were corrected;
only final successful runs count. No full concurrent source freeze.

Fresh inventory: **673 added / 336 named observations / 337 without**,
including concurrent lane gains. **Lane F 117 → 113 uncovered**: CLI 65,
SVG six, layouts 31, JS CI two and PNG/report nine. The synthetic empty-board
observer earns no original-function credit. Next capture remaining direct
SVG validation or CLI contracts and prioritize demonstrated failures.

Cleanup retires **17 superseded executable paths / 368.16 MiB allocated
path footprint**, with hashes, replacement/rebuild records and explicit
historical-binding retirement in `run72-cleanup.json`. Original corpus
inputs/expectations and negative witnesses remain. Verified Applesauce
**0.5.30 / LZFSE** saves **162.67 MiB across 24 files**; hashes, modes,
ownership and original non-compression xattrs verify, clone backups are
removed. Exact targets and rollback commands: `run72-compression.json`.
No bulk moved; figures are path/operation measurements during concurrent work.

Final allocated/apparent GiB: checkout **23.08/39.14**, `.reference` **11.90/24.91**, `target` **9.72/12.58**; **93.84 GiB free**. Every ceiling passes (`run72-disk-final.json`; six independent du scans exit zero). Hidden paths/.git and every apparent hard-linked path are included. Measurements are non-atomic during concurrent lanes. No builds or parity captures follow this audit.

### Run 71: native WASM isometric compile/render (2026-10-25)

Captured both original master `d2js/d2wasm/isometric_test.go` functions with
all assertions active. Fixed ignored isometric options, resolved compile
options/config hashing, SVG dispatch and incompatible-mode error admission.
Source: **functions.go:294–296,330–350,430–432,458–470** and
**d2lib/d2.go:109–135**. Reused the existing native renderer; the CLI and WASM
boundary now share the SVG wrapper from **d2svg/isometric.go:99–218**.
WASM-only renderer clocks use the existing web-time dependency; native clocks
retain their previous implementation.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Two original functions / complete observed calls | **1/12 → 12/12** | **0** |
| Prior WASM admission calls | **5/5 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Isometric CLI source contracts | **50/50 → unchanged** | **0** |
| WASM direct target golden guard | **936/952 → unchanged** | **0** |

All 966 e2e and 952 WASM-guard classifications remain unchanged, including
three empty-board entry differences, 11 stale Go matches and two Go decode
errors. Four compile calls compare complete diagram/render-options projections;
eight render calls compare full SVG bytes or typed error envelopes. Graph/FS
projection and rebuilt browser/Node package execution remain unqualified.
**102 native + 19 harness tests pass**, plus the WASM release target check,
TypeScript check, JS syntax and scoped formatting/lint checks. No additional
golden-board gain or complete API parity claimed. Other full tiers are inherited.

The first Go capture timed out after test PASS and remains non-passing.
Disabling Go's test-log output completed the original tests and process exit;
no original assertion or renderer output was removed. Captures retain streaming
**1 GiB total / 128 MiB file / 20 GiB free** guards and finite deadlines.
Evidence: `.reference/codex-runs/run71-{deltas,source}.json`, successful
`run71-wasm-isometric-qualified`, negative/final native reports and fresh guards;
full reproduction and file list in **report-run71.md**. No commits initiated.

Fresh inventory: **673 added / 324 named observations / 349 without**, including
concurrent lane gains. **Lane F 119 → 117 uncovered**: CLI 65, SVG ten,
layouts 31, JS CI two and PNG/report nine. No applicability exclusions.
Next capture remaining SVG isometric dispatch or CLI contracts, selecting
actual failures; the published WASM package still needs rebuilding/execution.
Cleanup removes **248.05 MiB** of allocated path footprint; verified current
retained LZFSE savings are **193.11 MiB**. Exact deletion/replacement records
and compression validation are in the run report.

Final allocated/apparent GiB: checkout **23.81/39.96**, `.reference` **12.56/25.38**, `target` **9.79/12.93**; **91.55 GiB free**. Every ceiling passes (`run71-disk-final.json`; six independent du scans exit zero). Hidden paths/.git and every apparent hard-linked path are included. Measurements are non-atomic during concurrent lane work. No builds or parity captures follow this audit.

### Lane I29: cooperative route-lane admission (2026-10-25)

Read the full brief through **Update 33**, including the initial Update.
Fixed `TestNativeLanesAdmissionAndCancellation`: native lane resolution dropped
the caller's context and hard-coded its work budget. Added an explicit
context/limit entry, sharing the bridge work counter's cancellation checks;
the edge pipeline supplies the mesh builder's context. Source:
**route_lanes.go:43–75**. Background wrappers remain; geometry is unchanged.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / calls | **7/8 → 8/8; 25/28 → 28/28** | **0** |
| Complete isometric SVG / expected errors | **870 / 138 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All **1,008 isometric** and **966 e2e classifications** repeat. The native
isometric probe is rebuilt; ordinary e2e uses the frozen I25/F69 CLI.
**29 native + 12 harness tests pass**, including all **20 PNG goldens**;
all six warm executables repeat. Explicit prior guards: header/bridge
**12/12**, surface **24/24**, content **20/20**. The latter two explicitly
execute the ordinary run's two ignored external-capture tests. Other full
tiers are inherited. No new golden gain or complete API parity claimed.

Original Go assertions remain active; full ordered paths, packet lengths,
errors and source records compare, including nonfinite coordinates/nil slices.
Native tests also check cancellation after work begins and before invalid
route validation in the pipeline. Replay is test-only; qualified requests
carry only inputs. Go source files are restored. Corrected incomplete-hook,
nil-slice adapter and concurrent shared-runner attempts never count as passes.
The demonstrated failure took priority over widening. Fresh inventory:
**673 added / 324 named / 349 without**. Lane I image **193 → 185 uncovered**,
plus the scene URL/Userinfo handoff: **186 total**. No applicability exclusions.
Next prefer remaining endpoint/arrow visibility and route/caption contracts.

Evidence: `.reference/codex-runs/iso29-{deltas,scope-final,source}.json`,
qualified capture, before/final requests/results, full SVG/e2e reports and
`iso29-verify.py`; alias `isometric-lanes-api.json`. The verifier checks the
retained input/expected-output closure, including **1,267 pinned SVG-tier
inputs** and all original error witnesses. Full reproduction/file list:
`report-iso29.md`. Streaming limits remain **1 GiB total / 128 MiB file /
20 GiB free**, with finite deadlines. No commits; other-lane work preserved.

Cleanup removes **1788.20 MiB allocated path footprint** in ten superseded
executables/Cargo variants and 1810 duplicate success artifacts;
original input/expected-output closure and negative witnesses remain
(`iso29-{retirement,cleanup}.json`). Verified Applesauce **0.5.30 / LZFSE**
saves **62.60 MiB across 34 operations**, preserving hashes, modes,
ownership and original xattrs; clone backups are removed. Exact rollback
commands are in `iso29-{compression,final-compression}.json`. No bulk moved;
savings are path/operation measurements during concurrent lanes. Historical
shared-runner bindings are distinct from replacement bytes.

Final allocated/apparent GiB: checkout **23.77/39.92**, `.reference` **12.52/25.34**, `target` **9.79/12.93**; **91.55 GiB free**. Every ceiling passes (`iso29-disk-final.json`; six independent complete du scans exit zero). Hidden paths/.git and every apparent hard-linked path are included. Measurements are non-atomic during concurrent lane work. No builds or parity captures follow this audit.

### Lane I28: crossing-route context and header contracts (2026-10-25)

Continued through brief **Update 32**. A combined capture of six header and
six crossing-route functions exposes the missing native context/work-limit
entry. Added `native_resolve_bridge_routes` and handed the mesh builder's
context through the edge pipeline; every work debit checks cancellation
before exhaustion. Source: **route_bridges.go:36–85; geometry_routes.go:342**.
Default wrappers remain. Cancellation within the preceding lane resolver
is still separate. The header algorithms already match.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions / operations | **11 matches + 1 unsupported → 12 matches; 40 + 3 unsupported → 43 matches** | **0** |
| Complete isometric SVG / expected errors | **870 / 138 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All **1,008 isometric** and **966 e2e classifications** repeat. The isometric
probe is freshly rebuilt; ordinary e2e uses the frozen I25/F69 CLI. **26 native
+ 12 harness tests pass**, including all **20 PNG goldens** and cancellation
checks after work begins and through the edge pipeline. Six compressed native
executables repeat all 26 tests. Prior precision is **6/6** and L30 surface
**24/24 / 2,910 operations**, explicitly invoking its ordinarily ignored
external-capture test. Other full tiers are inherited; no new golden gain or
complete API parity claimed.

All original Go assertions remain active; complete ordered paths, header
placement, errors and source records compare. Private replay compiles only in
tests. Original Go files are restored. Corrected mutable-observer and premature
shared-runner attempts are not passing evidence. Eight I25 visibility functions
were removed from the new batch; the demonstrated failure took priority over
widening. Fresh inventory: **673 added / 294 named / 379 without**. Lane I image
**205 → 193 uncovered**, plus the scene URL/Userinfo handoff: **194 total**.
No applicability exclusions. Next prefer route-lane admission/cancellation
and endpoint/arrow visibility contracts.

Evidence: `.reference/codex-runs/iso28-{deltas,scope-final,runner-bindings}.json`,
qualified original capture, before/final native results, full SVG/e2e reports,
test logs and `iso28-verify.py`. Alias: `isometric-geometry-api.json`.
Limits: **1 GiB total / 128 MiB file / 20 GiB free**, streaming guards and
finite deadlines. Full file list/rebuild commands: `report-iso28.md`.
No commits; other-lane edits and `29` preserved.

Cleanup retires **1,846 files / 1,369.59 MiB allocated path footprint**, with
pinned golden hashes and reproduction records replacing duplicate SVGs and
surface replays (`iso28-cleanup.json`). Original input/expected-output closure,
negative results and error witnesses remain; no bulk moved. Verified Applesauce
**0.5.30 / LZFSE** saves **248.71 MiB across 46 operations**, preserving hashes,
modes, owners and original xattrs; clone backups are removed. Exact paths and
rollback commands: `iso28-{headroom,final}-compression.json`. Path savings are
not concurrent net-volume attribution. A later lane replaced one shared runner;
`iso28-concurrent-rebuilds.json` retires that historical binding explicitly.

Final allocated/apparent GiB: checkout **23.25/39.17**, `.reference` **11.97/24.64**, `target` **9.81/12.89**; **91.17 GiB free**. Every ceiling passes (`iso28-disk-final.json`; six independent complete du scans exit zero). Hidden paths/.git and every apparent hard-linked path are included. Measurements are non-atomic during concurrent lane work. No builds or parity captures follow this audit.

### Lane L30: icon constructor cancellation and 24-function surface batch (2026-10-25)

Followed the complete brief through **Updates 31–32**, superseding the older
imgbundler scope. Captured **24 original functions / 2,910 ordered operations**
from text, rich-label, icon, density and vector-surface tests with every Go
assertion active. Complete texture pixels, layouts, optional retained SVG,
inline-link metadata, errors and texture reuse identities compare exactly;
direct allocation/viewport/icon-layout calls retain every original iteration.

`TestSurfaceIconCancellationAndBudgets` exposes one admission gap: native icon
construction accepted an already-cancelled context. Added
`SurfaceIconPainter::new_with_context`, checking cancellation before count
admission/resolver allocation (**surface_icons.go:59–66**), and passed the
renderer context at its existing caller. No raster or paint algorithm changes.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original functions | **23/24 → 24/24** | **0** |
| Complete observations | **2,909/2,910 → 2,910/2,910** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

**23 native + 18 harness + 38 raster-gate tests pass**, plus the external
capture replay. Current-source native guards cover five SVG / three asset
fixture boards and all **20 PNG goldens**. The 966 e2e classifications repeat
with the frozen, hash-verified L27 CLI. Other full tiers, including all 870
isometric SVG boards, are inherited. Private Go cache assertions and vector
fragment reimport tolerances remain original Go assertions; no full API or
additional golden-board gain is claimed. Native replay/private budget setters
compile only in tests, and no expected output enters the native implementation.

Fresh inventory: **673 added / 282 named observations / 391 without** at this
audit. **Lane L: 76 → 52 uncovered**, out of 111 assigned added functions,
with no applicability exclusions. Next batch: remaining raster admission/AA
and surface/vector contracts, fixing demonstrated failures before widening.
Evidence: `.reference/codex-runs/lang30-{deltas,source,verification}.json`,
original `lang30-surface-capture-final`, negative witness and qualified report;
alias `isometric-surface-api.json`. Full reproduction/file list:
`.reference/codex-runs/report-lang30.md`. Streaming capture limits remain
**1 GiB total / 128 MiB file / 20 GiB free**, with finite deadlines.

The first capture's empty dimensions observation and bridge setup errors
were corrected and never counted as passes. Shared executables were replaced
by concurrent Lane I builds; historical bindings are explicitly retired.
Its interim geometry-fixture recheck failure is preserved separately from
this lane's passing qualification. After its owner corrected the fixture,
**all six current executables / 24 tests pass**, including the 23 Lane L guards
and the added geometry test; no test was excluded. See
`lang30-compressed-tests-final/results.json`. No commits; other lanes preserved.

Cleanup removes **95 files / 1,557.25 MiB allocated path footprint** and seven
empty directories, retaining the original input/expected-output closure and
negative witness (`lang30-cleanup.json`). Verified Applesauce **0.5.30 / LZFSE**
saves **445.85 MiB across 38 files**, with hashes, modes, owners and original
xattrs verified; clone backups are removed. Exact rollback commands are in
`lang30-compression.json`; six later shared-build replacements have explicit
retirement records. No bulk moved; savings are operation measurements during
concurrent lane work.

Final allocated/apparent GiB: checkout **23.00/38.86**, `.reference` **11.79/24.46**, `target` **9.75/12.76**; **94.99 GiB free**. Every ceiling passes (`lang30-disk-final.json`; six independent scans exit zero). Hidden paths/.git and every apparent hard-linked path are included. Measurements are non-atomic during concurrent lane work. No builds or captures follow this audit.

### Lane L29: native mesh dynamic-overlay entry (2026-10-25)

Re-read the complete brief through **Update 31**, including the initial Update,
and newest reports/messages. Update 31 supersedes the older language/image-asset
assignments and splits isometric-image work by test file. Lane L begins with
**83 uncovered functions across 111 assigned master-added functions**.

Captured seven original `raster_test.go` / `raster_multiply_test.go` functions,
keeping all Go assertions active. Five static functions already match; two
functions need the native dynamic-overlay entry previously exposed only to
internal animation. `img/raster.rs::render_mesh_with_dynamic` forwards to the
existing renderer with static-camera fitting, dynamic depth testing and an
explicit context. Source: **raster.go:148–257,260–308**. It returns owned pixels;
retained preparation, cache performance and concurrent Frame calls remain
separate API work. No raster algorithm change or new golden-board gain.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Seven original raster functions | **5 matches + 2 unsupported → 7 matches** | **0** |
| Complete constructor/frame observations | **28/28 final** | **0 prior matches lost** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All **966 e2e classifications** repeat using the hash-verified frozen L27 CLI;
ordinary CLI paths are unchanged. **21 current-source native + 14 harness tests
pass**, including five SVG / three asset fixture boards and all **20 PNG
goldens**. All 21 native tests and 28 API calls repeat after verified compression.
The full 870 isometric SVG tier and other complete language/export/CLI/flag/watch
tiers remain inherited. Captures compare complete ordered premultiplied pixels,
dimensions, strides and errors; missing/corrupt/incomplete data fails. The
private texture-sampler scalar assertion remains an original Go assertion,
separate from the observed frame calls. Arbitrary Go image storage is unqualified.

Fresh inventory: **673 added / 248 named observations / 425 without** at this
audit. **Lane L: 83 → 76 uncovered**, with no applicability exclusions. Observation
coverage does not establish complete APIs. Next prefer the remaining raster
admission tests and text/icon resource limits, capturing actual failures first.

Evidence: `.reference/codex-runs/lang29-{deltas,source,verification}.json`,
original `lang29-raster-capture-final`, negative/final/compressed native reports,
fresh e2e guards and compressed native tests. Alias: `isometric-raster-api.json`.
Capture limits: **1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.
Shared build-lock waits and corrected verification-helper setup failures never
count as passing. No commits; other lanes and `29` remain untouched.

Cleanup retired **11 verified duplicate files / 2.72 MiB allocated path footprint**
and six empty directories; canonical inputs/expected outputs and negative results
remain (`lang29-cleanup.json`). Warm shared-test overwrites and the superseded
baseline runner have an explicit overlay (`lang29-retirement.json`). Verified
Applesauce **0.5.30 / LZFSE** saves **43.21 MiB across 22 files**, preserving hashes,
modes, owners and original xattrs; clone backups are removed. Exact targets and
rollback commands: `lang29-compression.json`. The observed target overage stopped
further builds/captures until a six-scan whole-checkout recovery audit passed.
Savings are path/operation measurements, not concurrent net physical-volume savings.
Final allocated/apparent GiB: checkout **23.09/38.27**, `.reference` **11.65/23.92**, `target` **9.98/12.71**; **95.75 GiB free**. Every ceiling passes (`lang29-disk-final.json`; six complete independent du scans exit zero). Hidden paths/.git and all apparent hardlinks are included. Measurements are non-atomic during concurrent lane work. No builds or captures follow this audit.

### Lane I27: SVG fragment retention and precision observations (2026-10-25)

Read the entire brief, including its initial Update and the new **Update 31**,
plus the newest reports and STATUS. The file-based split leaves Lane I **211
uncovered image functions** initially, rather than I26's former combined queue.
Captured six original functions from `svg_fragments_test.go`,
`svg_precision_test.go` and `svg_polygon_precision_test.go`: **36 operations,
all matching**, with every original Go assertion retained. No production
algorithm fix or whole-board gain is claimed. Static source mapping:
`svg_fragments.go:33–114`, `svg_visibility.go:85–105,209–262,290–295` and
`svg.go:119–218` → `img/svg_visibility.rs` and `img/svg.rs`.

The fragment observer compares the complete live index, spatial cells,
references, work and rejection errors before/after removal and re-addition.
Precision observations compare predicate IEEE-754 bits, ordered intersection
coordinates, complete path bytes and unchanged source polygons. Map IDs are
sorted; allocation addresses remain unqualified. The Rust replay is entirely
`cfg(test)`, with no new public API or production diagnostic construction.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Six original functions / operations | **6/6 / 36/36 → unchanged** | **0** |
| Prior atomic paint-order functions / calls | **6/6 / 12/12 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All **966 e2e classifications** stay unchanged using the frozen I25/F69 CLI;
other full tiers, including 870 isometric SVGs, are inherited. **22 freshly
built native + nine harness tests pass**. Native guards include all 20 retained
PNG goldens; all 22 tests and 36 operations repeat after verified compression.
Go test files are restored; scoped Rust/Go formatting, Ruff and diff checks pass.

Fresh inventory: **673 added / 254 named observations / 419 without**.
Update 31 Lane I image coverage: **211 → 205 uncovered**, plus the existing
scene URL/Userinfo handoff (**206 total**); Lane L's image files are excluded
from this ownership count, not from the repository inventory. No applicability
exclusions. Next prefer the remaining precision or native-edge contracts not
independently established by whole-board goldens.

Evidence: `.reference/codex-runs/iso27-{verification,scope-final}.json`,
original `iso27-precision-capture`, before/final/compressed reports, native
test logs and fresh e2e reports. Alias: `isometric-precision-api.json`.
Capture guards: **1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines.
Shared test-executable replacements are recorded in `iso27-warm-retirement.json`.
The initial missing-uv launcher was corrected; it is not passing evidence.
No commits; other lanes and `29` preserved. Exact file/rebuild list and final
disk audit: `report-iso27.md`.

Cleanup removed **two disposable launcher directories / 8 KiB allocated** (`iso27-cleanup.json`), preserving all corpus inputs and expected outputs. Verified Applesauce **0.5.30 / LZFSE** operations save **101.67 MiB across 28 files**, with checksum/mode/owner/xattr validation and all clone backups removed. Exact paths/rollback commands are in `iso27-{headroom,shared-headroom,final}-compression.json`. A sixth selected shared archive disappeared during concurrent cleanup; five completed validated operations were recovered from the retained log. Later shared-archive replacements are explicit in `iso27-compression-retirement.json`; operation savings are not net concurrent-volume savings.

Final allocated/apparent GiB: checkout **23.82 / 39.14**, `.reference` **12.45 / 24.73**, `target` **9.90 / 12.76**; **93.84 GiB free**. Every ceiling passes (`iso27-disk-final.json` and raw log); six independent complete du scans exit zero without errors. Hidden paths/.git and every apparent hard-linked path are counted. Measurements are non-atomic during concurrent lane work. No builds or parity captures follow this audit.

### Lane L28: assigned queue audit and regression guard (2026-10-25)

Re-read the full brief through **Update 30**, including its initial Update,
and the latest L27/I25/F69 reports/messages. Update 30 supersedes the older
imgbundler assignment. A fresh inventory confirms **673 added / 235 named
observations / 438 without**, with **zero uncovered functions** in Lane L's
six assigned packages: d2ir 45, d2compiler 29, d2lib ten, textmeasure nine,
d2themes six and d2parser three. These are observation counts, not complete
API parity. No other-lane work or applicability exclusions were selected.

The frozen L27 CLI repeats **933 SVG + 11 expected-error matches**; every
one of **966 e2e classifications** is unchanged (`fixed 0 / regressed 0`).
The seven original library source calls still match, and all **seven warm
source/config native tests** pass. No builds, original Go recapture or
production changes; other full scoreboards remain inherited. Older library
layout/reuse calls remain unsupported. Further implementation needs a new
Lane L assignment; no new golden-board gain is claimed.

Evidence: `.reference/codex-runs/lang28-{inventory,scope,deltas,verification}.json`,
fresh `lang28-e2e-run`, source replay and warm-test logs; reproduction and
the complete file list are in `report-lang28.md`. Fresh writable outputs use
**1 GiB total / 128 MiB file / 20 GiB free** guards and finite deadlines.
Cleanup removed two verified duplicate replay files (**8 KiB allocated /
5,030 bytes apparent**) and two empty temporary directories; the canonical
L27 input/output closure remains (`lang28-cleanup.json`). Verified Applesauce
**0.5.30 / LZFSE** saves **360 KiB across three files**, preserving hashes,
modes, owners and original xattrs; clone backups are removed. Exact paths
and rollback commands: `lang28-compression.json`. No bulk moved. Final
allocated/apparent GiB: checkout **22.91/38.06**, `.reference`
**11.61/23.89**, `target` **9.84/12.53**; **97.27 GiB free**.
Every ceiling passes (`lang28-disk-final.json`; six complete independent scans,
including hidden paths/.git and all apparent hardlinks). No builds/captures
follow the audit. No commits; other lanes and `29` preserved.

### Run 70: WASM render admission and current-pin regressions (2026-10-25)

Read the full brief through arriving Updates 31–32, newest reports/messages
and STATUS; incorporated concurrent I/L evidence. Four original WASM
functions expose five calls. Source comparison found full-tree validation
ran after child removal, appendix animation was accepted, and render errors
lost one wrapper. The checked boundary now follows
**d2wasm/functions.go:351–366,398–405,505–540** and preserves typed 400/500
errors. `render_d2_target_response_json` exposes the data/error envelope for
already-decoded arguments. Direct target-render errors retain their existing
contract; no geometry or SVG serialization changed. The WASM source renderer
also supplies two previously missing default fields, restoring compilation.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Four original functions / five calls | **1/5 → 5/5** | **0** |
| Prior direct raw-target calls | **101/101 → unchanged** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| WASM target golden guard | **936/952 → unchanged** | **0** |

All 966 e2e and 952 WASM-guard classifications stay unchanged. The latter
keeps three empty-board entry-point differences, 11 stale Go matches and two
Go decode errors visible; no additional golden gain is claimed. The original
Go WASM assertions execute in Node; native Rust compares complete responses,
error messages/codes and SVG bytes. Raw request argument coercion, Rust/WASM
transport/package rebuilding and isometric WASM remain unqualified. Other
full language/raster/CLI/flag/watch tiers are inherited.

**101 native + nine harness tests pass.** The full native run initially found
three v0.9.0 assertions inconsistent with master. An independent current-pin
Go observer re-evaluated **105 unchanged regression inputs**: 95 direct targets
(72 complete errors / 23 complete SVGs), eight ASCII calls and two UTF-16
completions. The tests now compare complete current results; historical input
fixtures and negative baselines remain intact. This adds no master Test-name
credit. All 101 tests repeat from verified compressed executables. Production
runners rebuilt after the fixture refresh are byte-identical
to the frozen binaries used by the guards. Scoped formatting, Ruff and diff
checks pass. Setup/compile failures remain diagnostic history, not passing
results. Shared Cargo lock waits are logged.

Evidence: `.reference/codex-runs/run70-{deltas,source,verification}.json`,
original WASM capture, before/final requests/results, native tests, fresh e2e
and WASM guards. `run70-refresh-legacy.py` reproduces the independent fixture
refresh. Capture limits: **1 GiB total / 128 MiB file / 20 GiB free**, streaming
guards and finite deadlines. No full concurrent checkout freeze is claimed.
Fresh inventory: **673 added / 282 named observations / 391 without** at this
audit, including concurrent gains. **Lane F 123 → 119 uncovered**: CLI 65,
SVG ten, layouts 31, WASM/JS four, PNG/report nine. No applicability exclusions.
Next capture remaining SVG isometric dispatch or the two WASM isometric
functions, then choose demonstrated failures. No commits initiated; other
lanes and `29` preserved. Supervisor paths and storage evidence: `report-run70.md`.

Permanent cleanup removed **281.96 MiB allocated path footprint**, preserving
all corpus inputs and expected outputs. Failed warm executable replacement is
recorded separately and excluded from savings; two shared Cargo files were
reinstated at their original hashes and remain required cache. Verified
Applesauce **0.5.30 / LZFSE** saves **201.21 MiB across 23 files**; hashes,
modes, owners and original xattrs verify, and clone backups are removed.
Ledgers: `run70-{retirement,cargo-retirement,setup-cleanup,storage-savings}.json`
and `run70-*-compression.json`. No bulk moved; savings are path/operation figures.

Final allocated/apparent GiB: checkout **22.94/38.80**, `.reference`
**11.73/24.40**, `target` **9.75/12.76**; **95.37 GiB free**. All ceilings
pass (`run70-disk-final.json`); six independent complete scans exit zero,
include hidden paths/.git and count every apparent hardlink. Measurements are
non-atomic during concurrent lanes. No further builds or parity captures follow
this audit.

### Run 69: SVG label serialization and LaTeX admission (2026-10-25)

Read the complete brief through Update 30 and the newest reports/messages.
The next five original renderer functions expose **ten calls**: shape/edge
classes, special-shape IDs, SQL constraints and excessive LaTeX nesting.
Static comparison found three emission gaps: group classes bypassed attribute
escaping (**d2svg.go:1079–1089,1707–1723**); normal/sketch SQL constraints
bypassed `SetText` (**table.go:82–111; d2sketch/sketch.go:536**); and LaTeX
admission ran inside the parse-error wrapper (**latex_common.go:8–15**).
The native renderer now uses shared attribute/text serialization and checks
LaTeX limits before wrapping parser errors. No fixture-specific output.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Five original functions / ten calls | **4/10 → 10/10** | **0** |
| Prior raw-target admission calls | **91/91 → unchanged** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Shared-clock cli-diff | **119/119 → unchanged** | **0** |
| Strict release/shared-clock CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |

All 966 e2e outcomes and CLI case/golden classifications are unchanged.
All original Go assertions stay active; complete SVG bytes, nil outputs and
errors compare without normalization. **271 native + 21 harness tests pass**;
all native tests and ten calls repeat after verified compression. Other
language, raster, flags, watch and isometric tiers are inherited. No additional
golden-board gain or complete renderer API parity is claimed.

Evidence: `.reference/codex-runs/run69-{deltas,source,compression-recheck}.json`,
original `run69-svg-capture`, negative/final calls and fresh e2e/CLI guards;
`run69-verify.py` checks original fixture hashes, own bindings, Go restoration
and compressed captures/runners. Alias: `svg-render-labels.json`. Capture limits
remain **1 GiB total / 128 MiB file / 20 GiB free**, with finite deadlines.
Inventory: **673 added / 220 named observations / 453 without** at this run's
audit. **Lane F 128 → 123 uncovered**: CLI 65, d2svg ten, layouts 31, WASM/JS
eight, PNG/report nine. No applicability exclusions. Next prefer remaining SVG
isometric dispatch or WASM raw-target admission after capturing actual failures.
No commits; other lanes and `29` preserved. Supervisor file list, rebuild
commands and storage ledger: `report-run69.md`.

Cleanup retired **two superseded F68 executable snapshots / 31.34 MiB allocated path footprint**; hashes, rebuild instructions and replacements remain in `run69-retirement.json`. Historical shared-test overwrites are recorded separately. No corpus inputs or expected outputs were removed. Verified Applesauce **0.5.30 / LZFSE** saves **109.55 MiB across 36 files**; checksums, modes, ownership and original xattrs verify, and clone backups were removed (`run69-final-compression.json`). Savings are path footprints, not concurrent net physical-volume savings.

Final allocated/apparent GiB: checkout **22.60/37.95**, `.reference` **11.38/23.66**, `target` **9.77/12.72**; **97.66 GiB free**. Every ceiling passes (`run69-disk-final.json`, raw du/df); all six scans exit zero. Hidden paths/.git and all apparent hard-linked paths are included. Measurements are non-atomic during concurrent lane work. No further builds or parity captures follow this audit.

### Lane I26: atomic SVG paint-order observations (2026-10-25)

Continued the complete brief through **Update 30**, newest lane reports and
STATUS. Captured all six original `d2isometricimg/svg_order_test.go:25–143`
functions (**12 calls**), retaining every Go assertion. All calls already
match `img/svg_order.rs`; no production algorithm change or new golden-board
gain is claimed. Source mapping: **svg_order.go:18–197**.

The observer compares the complete painter permutation, unit/batch identity
relations, authored opacity/depth/source indices, every polygon coordinate,
errors, cancellation and consumed ordering statistics. The fragmented-surface
test retains its **1,200 fragments per surface / 400,000 work** contract. Rust borrows an
immutable ordering view and returns a permutation. Allocation addresses,
unused batch rendering fields and statistics discarded by original error-only
calls remain unqualified; no zero statistics are fabricated on native errors.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Six original functions / 12 calls | **6/6 / 12/12 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

Both e2e runs preserve all **966 outcomes**, using the unchanged frozen F69
CLI copy from I25. Other full scoreboards, including 870 isometric SVGs, are
inherited. **19 freshly rebuilt native + seven harness tests pass**. The harness rejects
missing/extra calls and changed order, opacity, batch identity, coordinates,
stats or errors. The compressed native replay repeats all 12 calls, and all
19 native tests repeat after compression. Preserved I25 warm tests also pass;
four subsequently overwritten shared executable bindings have an explicit
retirement overlay.
Original Go files are restored. Rust/Go formatting, Ruff and scoped diff checks
pass. The initial e2e launcher used a relative path under the trace cwd;
the corrected absolute-path run passes. A fresh native test build initially
hit Lane F's transient renderer module-reference error; the corrected retry
passes, separately from the preserved warm-test evidence.

Fresh inventory: **673 added / 241 named observations / 432 without**.
Lane I **301 → 295** uncovered: image **294**, scene **one**, the existing
URL/Userinfo ownership handoff. No applicability exclusions. Next prefer
bounded live-fragment references (`svg_fragments_test.go`) and the precision
contracts that whole-board goldens cannot independently establish; GIF stays
with Lane F.

Evidence: `.reference/codex-runs/iso26-{verification,retirement}.json`, original
`iso26-order-capture`, before/final/compressed requests/results and fresh e2e
guards. Alias: `isometric-order-api.json`. Capture budgets remain **1 GiB total /
128 MiB file / 20 GiB free**, with finite deadlines. No complete source freeze
of concurrent CLI/render changes. I26's runner file was included in the
supervisor's concurrent `2b53e4b6` commit; this lane initiated no commits.

Cleanup retired **three idle superseded CLI snapshots / 81.33 MiB allocated**;
source/rebuild records and the verified F69 replacement remain in
`iso26-retirement.json`. No corpus inputs, expected outputs, API runners or negative
baselines were deleted. Verified LZFSE saves **41.27 MiB across 16 files**, preserving
bytes/modes/owners/xattrs with all clone backups removed. Exact targets/rollback
commands: `iso26-compression.json`, `iso26-final-compression.json`. Final
allocated/apparent GiB: checkout **23.01/38.09**, `.reference` **11.64/23.92**, `target`
**9.85/12.55**; **95.37 GiB free**. Every ceiling passes (`iso26-disk-final.json` and
raw du/df); both complete scans exit zero without errors. Hidden paths/.git and all
apparent hard-linked paths are counted. Measurements are non-atomic during concurrent
lane work. No further builds or parity captures follow this audit.

### Lane I25: visibility and cubic-route observations (2026-10-25)

Read the full brief through Update 30 and the newest lane reports; incorporated
arriving F69/L27 observations. Captured all eight original functions in
`d2isometricimg/svg_visibility_test.go` (**27 calls**) and both public calls in
`TestContainerHierarchyCubicRouteIsSampledFaithfully`. Every original Go
assertion remains active. All **nine functions / 29 calls already match**;
no production algorithm change or new golden-board gain is claimed.

The visibility observer compares complete ordered fragments and errors,
cancellation, work allowances, and group/owner equality. Nonfinite input
coordinates use explicit scalar tags; none are erased by JSON encoding.
The cubic observer compares the complete scene/source and malformed-control
error against `hierarchy.go:139–204`. Its direct private shared-budget call
remains a Go assertion and is tested separately in native `scene.rs`; that
call is not claimed as a cross-language observation. Allocation addresses
and the existing URL/Userinfo limitation remain unqualified.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Visibility functions / calls | **8/8 / 27/27 → unchanged** | **0** |
| Cubic function / public calls | **1/1 / 2/2 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All **966 e2e outcomes** stay unchanged. The first guard used frozen F68;
after its concurrent retirement, the final guard used a dedicated F69 copy.
The failed missing-executable attempt is not passing evidence. Other full
scoreboards, including 870 isometric SVGs, are inherited. **19 native + six
harness tests pass**, including whole SVG/PNG/assets and the private point
budget. All 19 tests and both API replays repeat from compressed artifacts.
Scoped formatting, Ruff and diff checks pass; Go tests are restored.

Fresh inventory: **673 added / 235 named observations / 438 without**,
including concurrent F69/L27 gains. Lane I **310 → 301** without named
observations: image **300**, scene **one** (`TestIconURLIdentityAndOwnership`).
No applicability exclusions. The URL case retains its I23 foundation handoff.
Next prefer an uncaptured admission/work-budget batch such as the six
`svg_order_test.go` functions, then fix demonstrated native failures.

Evidence: `iso25-{deltas,verification,inventory-current}.json`, original
visibility/cubic captures and final native requests/results. New aliases:
`isometric-visibility-api.json`, `isometric-cubic-api.json`. Capture guards:
**1 GiB total / 128 MiB file / 20 GiB free**, finite deadlines. Failed hook
setup captures and byte-identical successful replay duplicates were retired
only after verification, preserving the canonical I/O. Cleanup and the fresh
disk audit are recorded in `report-iso25.md`. Cleanup removed **16 files /
1.05 MiB allocated**; verified LZFSE saves **44.05 MiB across 20 files**,
preserving bytes/modes/owners/xattrs with all clone backups removed. Final
allocated/apparent GiB: checkout **22.66/38.34**, `.reference` **11.60/23.87**,
`target` **9.87/12.77**; **97.66 GiB free**. Every ceiling passes
(`iso25-disk-final.json`; complete du scans exit zero and count hidden paths
and apparent hardlinks). No more builds/captures follow the audit. No commits
or other-lane edits.

### Lane L27: library source admission; assigned queue complete (2026-10-25)

Followed the full brief through **Update 30**, L26's corrected handoff and
newest lane reports. Captured the last six original `d2lib/d2_test.go:18–91`
functions with every Go assertion active: seven calls covering Parse/Compile
cancellation, all four expansion limits and nil-FS imports.

`d2-rs-lib::source::{parse_source,compile_source_ir}` forwards source path,
UTF-16 positions, all limits, explicit import capability and the same context
through parsing and IR compilation. Source mapping: **d2lib/d2.go:26–47,68–106**,
**d2compiler/compile.go:64–99**, **d2parser/parse.go:52–100**. Complete errors,
typed cancellation and the complete partial parse tree match. Imports remain
disabled without an opener. Graph/layout/render and resolver/reuse APIs remain
separate; successful full-Compile operations are explicitly unsupported by
this adapter. Existing native algorithms are reused; no new golden-board gain
or full library API parity is claimed.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Six original functions / seven source calls | **7 unsupported → 7 matches** | **0** |
| Prior library observation | **19 matches / 10 unsupported → unchanged** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Language live goldens / source-error contracts | **915 / 445 → unchanged** | **0** |

All **966 e2e outcomes** stay unchanged with a rebuilt frozen CLI.
**74 native + nine harness tests pass** (source four, config three, warm
language 67; new harness three, shared harness six). Compressed source/config
executables, all seven original calls and the compressed CLI guard repeat.
The new error-chain test caught and fixed a context-wrapper cause loss.
Scoped formatting, lint and diff checks pass. Other full export/CLI/flag/watch/
isometric tiers are inherited; older library layout-reuse calls remain unsupported.

Fresh inventory: **673 added / 235 named observations / 438 without**, including
concurrent lane gains. **Lane L's Update 30 queue: 6 → 0 uncovered**. No
applicability exclusions. The final audit found two names shared across packages;
explicit `d2lib` package metadata now disambiguates them, with a harness
regression test. Native request/result bytes are unchanged. Further work needs
supervisor reassignment.
Evidence: `.reference/codex-runs/lang27-{deltas,verification}.json`, original
capture, negative/qualified/compressed requests/results and fresh e2e/language
guards. Alias: `library-source.json`. Seven original calls, original Go
restoration, **930 language golden hashes**, current runner hashes and all
**19 retired paths** verify. Budgets: **1 GiB total / 128 MiB file / 20 GiB
free**, streaming guards and finite deadlines. Rebuilds, file list and limitations:
`report-lang27.md`. No full concurrent shared CLI source freeze is claimed.

Cleanup removed **19 files / 81.42 MiB allocated path footprint**: three idle
superseded L23–25 CLI snapshots and a superseded initial capture. Complete
negative/current inputs and expected outputs remain. Verified Applesauce
**0.5.30 / LZFSE** saves **36.11 MiB across 12 files**, preserving hashes,
modes, ownership and xattrs; clone backups are removed. Exact deletion/replacement
and rollback ledgers: `lang27-{cleanup,retirement,compression,final-data-compression}.json`.
No bulk moved; savings are path/operation figures, not net volume savings.
Final allocated/apparent GiB: checkout **22.66/38.08**, `.reference` **11.38/23.66**, `target` **9.82/12.78**; **97.66 GiB free**. Every ceiling passes
(`lang27-disk-final.json`); all six scans exit zero, include hidden paths/.git
and count all apparent hard-linked paths. Measurements are non-atomic during
concurrent lane work. No further builds or parity captures follow this audit.
No commits; other lanes and `29` preserved.

### Lane L26: library configuration and LaTeX admission (2026-10-25)

Read the complete brief through Update 30, latest reports/messages and STATUS;
incorporated arriving I24/F68 evidence. Captured 29 original d2lib calls with
all original Go assertions active. **Four master-added functions / 17 calls
now match**: isometric precedence, unspecified config, disabled-mode SVG
identity and excessive LaTeX nesting. Two additional calls from the older
theme-precedence test match; ten older layout-reuse calls remain explicitly
unsupported by the native resolver/callback API. No applicability exclusions.

Static source comparison found compiler measurement bypassed master's LaTeX
admission. `d2-rs-math/input_limits.rs` ports **d2latex/limits.go:8–67** and
runs before recursive rendering/measurement. The graph measurer preserves
these errors without the MathJax parse prefix (**latex_common.go:10–30**).
Both original object/edge depth-129 calls were incorrectly accepted before;
now both return the complete Go error. The new `d2-rs-lib::config` boundary
ports **d2lib/d2.go:108–140,289–354**: source fills absent caller options,
then defaults apply; false/zero/empty palettes retain precedence, and disabled
isometric stays absent from the diagram's appearance config. The adapter uses
the native source/layout/render path; a full library resolver/reuse API is
still absent. Pointer/alias identity is not qualified.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Four master-added functions / 17 calls | **0/4 / 15 unsupported + 2 mismatches → 4/4 / 17 matches** | **0** |
| All ten observed functions / 29 calls | **0 matches → 5 matching functions / 19 matching calls; 5 functions / 10 calls unsupported** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Language live goldens / source-error contracts | **915 / 445 → unchanged** | **0** |

All 966 e2e outcomes remain unchanged. Four complete disabled-isometric SVGs
compare byte-exactly under the ordinary release version stamp; no SVG/error
normalization. **215 native + six harness tests pass**; all 215 native tests
repeat from compressed executables, as do the 19 supported API calls. Scoped
formatting/lint/diff checks pass. Other full raster/CLI/flag/watch/isometric
tiers are inherited; no new golden-board gain is claimed.

**Inventory correction:** L25's package total of ten was correct, but its
category handoff was not. The five layout-reuse functions and the direct
theme-precedence function already existed at v0.9.0. The actual master-added
remainder is **Lane L 10 → 6**, all in `d2lib/d2_test.go`: cancellation, four
expansion limits and nil-FS imports. These are applicable native contracts,
not Go-internal exclusions. Fresh whole-repository inventory: **673 added /
215 named observations / 458 without**, including I24/F68's concurrent gains.

Evidence: `.reference/codex-runs/lang26-{deltas,source,bindings,verification}.json`,
`lang26-source.patch`, original capture, negative/final/native replays and
fresh scoreboard guards. Alias: `library-api.json`. All **52 own I/O + 565
prior L25 I/O bindings and 930 original language-golden hashes** verify after
cleanup; original Go tests are restored. No full concurrent CLI source freeze
is claimed. Capture guards: **1 GiB total / 128 MiB file / 20 GiB free**, finite
deadlines. Supervisor paths and rebuild commands: `report-lang26.md`.

Storage: a concurrent test build briefly reached **10.04 GiB target**. Parity
work paused for scoped cleanup and verified warm-executable compression;
`lang26-disk-restored.json` proves every ceiling passed before resuming.
Cleanup removed **41 files / 9.99 MiB allocated path footprint**. Verified
Applesauce 0.5.30/LZFSE records **240.11 MiB saved** in 43 operations; four
additional verified writes lack savings measurements and are omitted. All
backup clones are removed. Ledgers: `lang26-{cleanup,retirement,storage-savings}.json`
and `lang26-*-compression.json`. Final allocated/apparent GiB: checkout **22.63/38.07**, `.reference` **11.35/23.63**, `target` **9.82/12.80**; **103.00 GiB free**. Every ceiling passes (`lang26-disk-final.json`); all independent scans exit zero, include hidden paths/.git and count apparent hardlinks. No further builds or parity captures follow this audit.
No commits; other lanes and `29` preserved.

### Lane L25: validated theme elements; Update 30 inventory (2026-10-25)

Read the full brief through arriving **Update 30**, newest lane reports and
STATUS. The initial netpolicy guard remains **8/8 functions / 39 operations**;
all 32 warm foundation tests and 18 netpolicy harness tests pass. Update 30's
fresh inventory finds IR **45/45**, compiler **29/29**, textmeasure **9/9** and
parser **3/3** already observed. Lane L's actual uncovered queue was **16**:
ten d2lib functions and six theme functions. Selected the shared serialization
failure in the six theme functions rather than repeating closed language work.

`d2-rs-render` ThemableElement now follows **d2themes/element.go:94–330** and
**lib/svg/markup.go:44–80,145–168**: ordered typed attributes escape once;
legacy attribute fragments/local references are validated; finite numbers and
typed text/trusted content retain their source boundaries. The image caller
passes its raw URL as **d2svg.go:1824** does. Public typed element helpers,
owned attribute copies and three composition regressions cover the new API.
Post-render gradient field mutation is not qualified by this observation tier.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Six original theme functions / 15 operations | **0/6 / 2 matches, 11 mismatches, 2 unsupported → 6/6 / 15 matches** | **0** |
| Fresh e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Raw SVG goldens | **939 matches, 11 stale in Go, 2 Go errors → unchanged** | **0** |
| Dark / appendix / sketch / exporter goldens | **12 / 6 / 38 / 19 → unchanged** | **0** |

All original Go assertions remain active. Full SVG bytes and panic admission
compare; Go panic text and object allocation identity are not claimed.
Before/final native request bytes are identical. All **966 e2e row outcomes**
remain unchanged. Sketch's one unbound golden and all 13 stale/error raw rows
remain non-passing. Exporter's two additional assertions pass.
**301 native + 25 harness tests pass**, including original SVG/theme fixtures
with 51/15 calls. Compressed runners repeat all 15 theme operations and the
CLI smoke. Scoped formatting/lint/diff checks pass. No commits or changes to
other lanes' code; the one raw-image href hunk in shared render.rs is Lane L's.

Fresh inventory: **673 added / 198 named observations / 475 without** at this
audit, including newly published I23/F67 reports. **Lane L: 16 → 10 uncovered,
all d2lib.** The original 143-function inventory is now fully observed after
I23; this does not establish every inline assertion or full API parity.
Evidence: `lang25-{deltas,verification,corpus-check}.json`, original capture,
negative/final native results, fresh guards and `lang25-final-inventory-qualified.json`.
New alias: `themes-api.json`. **565 L25 + 17 L24 + 423 L23 + 476 L22 + 475 L21**
I/O hashes and all **930 language golden hashes** verify after cleanup.
Capture guards remain **1 GiB total / 128 MiB file / 20 GiB free**, with finite
deadlines. Other raster/CLI/flag/watch/isometric tiers are inherited.
Handoff, source/rebuild details and explicit supervisor file list: `report-lang25.md`.

Cleanup actually deletes **16 files / 377.45 MiB allocated path footprint**,
with historical executable bindings explicitly retired and source/negative
witnesses retained. Verified Applesauce **0.5.30 / LZFSE** saves **35.34 MiB
on 11 idle files**, preserving bytes, modes, owners and original xattrs;
verified backup clones are removed. Exact ledgers/rollback commands:
`lang25-{cleanup,retirement,compression}.json`. No bulk moved.
Final allocated/apparent GiB: checkout **22.20/36.65**, `.reference`
**11.08/22.60**, `target` **9.65/12.40**; **103.76 GiB free**. Every ceiling
passes (`lang25-disk.json`); independent full scans exit zero without errors,
include hidden paths/.git and count apparent hardlinks. Concurrent measurements
are non-atomic; savings are path/operation figures, not net volume savings.
No further builds or parity captures follow this audit.

### Lane I24: scene admission and source layout contracts (2026-10-25)

Followed the complete brief through **Update 30**. Captured seven original
hierarchy/sequence functions (**19 calls**) and all **13 calls** in the
original `TestValidation`, preserving every Go assertion. Layout/sequence
results are **7/7**, before and after. Validation improves **9 → 11 exact
matches**, with **two mismatches fixed** and **two unsupported calls unchanged**.
Both failures were native-only invalid values: an infinite route coordinate
and a NaN content aspect ratio. `scene.go:344–399` checks these before encoding;
Rust's JSON-first budget erased them as null. `scene/admission.rs` now visits
native scalars directly, retaining source text/collection/numeric budgets.
Options are visited as a struct, without inventing collection entries.

Typed input mutations execute after capture of the valid source; neither
nonfinite numbers nor invalid UTF-8 are normalized through JSON. Nil diagram
references and invalid UTF-8 in native `String` remain explicitly unsupported.
Complete errors and nil-scene results compare. The existing ownership suite
remains **seven matches + one unsupported URL/Userinfo case**. No allocation
identity, full scene API parity or new golden-board gain is claimed.

Fresh guards retain **1,008/1,008 scenes**, **870 SVG goldens + 138 source
errors**, and **933 ordinary e2e SVGs + 11 expected errors**, with zero
regressions. All **966 ordinary e2e row outcomes** are unchanged; both ordinary
guards use the unchanged frozen F67 CLI, while scene/SVG guards use the newly
built native isometric probe. Other scoreboards are inherited. **Seven native
tests + five harness tests pass** (admission, complete SVG/PNG and assets).

Fresh inventory: **673 added / 215 named observations / 458 without**,
including concurrent F68/L26 observations. Lane I **318 → 310** without named observations: **image 308, scene two**
(`TestContainerHierarchyCubicRouteIsSampledFaithfully` and
`TestIconURLIdentityAndOwnership`). Validation has a named observation,
while its two unsupported calls remain non-passing. No applicability exclusions.
Evidence: `.reference/codex-runs/iso24-{deltas,source,inventory-sealed}.json`,
original captures, negative/final native results and `iso24-verify.py`
(**4,060 complete retained outputs**, pinned inputs, source/runner hashes
and original Go restoration). New aliases: `isometric-layout-api.json` and
`isometric-scene-admission.json`; full scene/SVG aliases are refreshed with
previous report bytes preserved. Scoped file list and final storage evidence
are in `report-iso24.md`. No commits. Cleanup retires **four obsolete
executables / 31.60 MiB**; verified LZFSE saves **785.52 MiB across 2,315
idle retained files**, preserving bytes/modes/owners/original xattrs and removing
clone backups. Three compressed API replays are byte-identical; seven compressed
native tests and post-compression closure verification pass. A transient
**32 MiB target overage** stopped new builds/captures; historical binding checks,
scoped retirement and fresh whole-checkout scans resolve the stop gate.
Final allocated/apparent GiB: checkout **22.63/38.07**, `.reference`
**11.35/23.63**, `target` **9.82/12.80**; **103.00 GiB free**. Every ceiling
passes (`iso24-disk-final.json`); measurements are non-atomic during concurrent
lanes. Savings are path/operation figures; no bulk moved.

### Lane I23: network policy and scene ownership observations (2026-10-25)

Read the complete brief through **Update 30**. Added original-assertion
observers for **TestHTTPNetworkPolicy** (one function / eight operations) and
eight scene functions (26 operations). Network is **1/1**; scene is **seven
matches + one unsupported**. Both qualified before/final deltas have **zero
regressions**. Complete construction scenes and source diagrams compare;
nine native typed mutations verify the original input remains unchanged.
HTTP uses real loopback sockets and compares complete bytes, metadata, errors,
unavailable flags, endpoint hits and cache state. Original Go assertions stay
active; Go source is restored. No production algorithm or golden fix claimed.

Fixed an actual native HTTP test-fixture failure: explicitly clear inherited
nonblocking mode on accepted sockets before `read_exact`. **Three HTTP + five
cache tests and three Python harness tests pass**. Compressed frozen replay
results are byte-identical; all eight compressed warm tests repeat. Both e2e
guards remain **933 SVG + 11 errors**, with all **966 outcomes unchanged**,
using the unchanged frozen F66 CLI. Other tiers inherited.

The scene pilot overobserved an internal label alias and lacked a lossless
Userinfo input. Its three mismatches are nonqualifying harness evidence,
not a production before baseline. The final observer compares the source
ownership contract those tests assert, without comparator normalization.
**TestIconURLIdentityAndOwnership stays explicitly unsupported**: captured
source icon strings retain credentials separately, while the native target
URL cannot represent them. Go restores these private fields explicitly
(**scene.go:161–200,329–343**). Foundation request: an owned URL/Userinfo
boundary that preserves authored identity while retaining opaque Go metadata
JSON. Native ErrBlockedAddress identity and internal allocation/alias identity
remain unqualified; the Go typed assertions remain active.

Fresh inventory: **673 added / 184 named observations / 489 without**.
Lane I **326 → 318** uncovered (**image 308, scene 10, imageasset 0**); the
original 143-function list now has named observations for all functions,
without claiming full API parity. No applicability exclusions. Evidence:
`.reference/codex-runs/iso23-{deltas,bindings,inventory-final}.json`, complete
source/native captures, tests and `iso23-verify.py` (**43 source/input/output
bindings**, original Go source restoration and runner checks). Aliases:
`imageasset-network-api.json`, `isometric-scene-api.json`. Scoped file list,
explicit pilot/warm-executable retirements, deletion/compression ledgers and
final disk audit are in `report-iso23.md`. No commits. Final allocated/apparent
GiB: checkout **22.42/36.82**, `.reference` **11.08/22.60**,
`target` **9.77/12.58**; **102.62 GiB free**. Every ceiling passes;
cleanup deleted **1.67 MiB**, verified retained-file compression saved **9.75
MiB** (path/operation figures; `iso23-disk.json` and ledgers).

### Run 68: raw-target SVG structural admission (2026-10-25)

Read the full brief through **Update 30**, including the initial Update and
newest reports/messages; incorporated L25 and arriving I24. Selected five
original d2svg functions / **31 calls** as the next shared admission cluster.
Static comparison found missing **d2target/render_validation.go:23–437** before
geometry and recursive hashing (**d2svg.go:2866–2905,3575–3635**). The native
renderer now checks the complete owned board tree, route/arrowhead and numeric
invariants, image icons, font families and projected legend fields. Empty
font names use bundled defaults (**d2svg.go:2492–2500**). The pinned ARM64
segment-length FMA order is verified from assembly and preserved.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Structural calls, five original functions | **4/31 → 31/31** | **0** |
| Prior paint/link calls | **60/60 → unchanged** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Shared-clock cli-diff | **119/119 → unchanged** | **0** |
| Strict release/shared-clock CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |

**27 failures fixed**, including one native crash. Original Go assertions stay
active; complete errors, nil outputs and SVG bytes compare without normalization.
All **966 e2e outcomes**, 119 cli-diff rows and CLI case/golden classifications
are unchanged. **270 native + 14 harness tests pass**; compressed warm tests
repeat all 270, and compressed input/output rechecks repeat 31/31 and 67/70 / 51
byte-identically. Go restoration, scoped formatting/Ruff and retained corpus
hash checks pass. Other tiers are inherited; no new board golden claimed.

Evidence: `.reference/codex-runs/run68-{deltas,source,compression-recheck}.json`,
original capture, negative/final native calls, fresh e2e/CLI reports and
`run68-verify.py`; alias `svg-render-structural.json`. Capture guards remain
**1 GiB total / 128 MiB file / 20 GiB free**, with finite deadlines. No full
source freeze of concurrent foundation/CLI edits is claimed. Fresh inventory:
**673 added / 215 named observations / 458 without** (including I24/L26).
**Lane F 133 → 128 uncovered**: CLI 65, d2svg 15, layouts 31, WASM/JS eight,
PNG/report nine. No applicability exclusions. Direct pointer cycles/nonfinite
values and arbitrary custom-font registration remain unqualified by these JSON
calls. Next prefer SVG isometric dispatch or WASM raw-target admission, selecting
actual failures after capture. No commits initiated.

Cleanup deleted **129 files / 31.56 MiB allocated path footprint**: superseded successful replay files, duplicate assembly output, a pending note and the idle F66 CLI snapshot. The snapshot's old hash, source/rebuild record and F68 replacement remain in `run68-retirement.json`; negative SVG runners and every input/expected output remain. `run68-warm-retirement.json` explicitly records two superseded Cargo warm-executable bindings. No bulk moved. Verified Applesauce **0.5.30 / LZFSE** saves **107.56 MiB across 35 operations**; hashes, executable modes, ownership and original xattrs verify, clone backups are removed. Exact paths/rollback commands: `run68-{runner,final}-compression.json`. Local APFS verified; DiskManagement inspection unavailable. Savings are path/operation figures, not concurrent net physical-volume savings.

A concurrent build briefly put target **27.16 MiB** above its ceiling. New parity runs paused; cleanup/compression and the complete `run68-disk-gate.json` audit proved every ceiling passed before compressed rechecks.

Final allocated/apparent GiB: checkout **22.63/38.07**, `.reference` **11.35/23.63**, `target` **9.82/12.80**; **103.00 GiB free**. Every ceiling passes (`run68-disk-final.json`, raw du/df; both full scans exit zero without errors). Hidden paths/.git and all apparent hard-linked paths are included. Measurements are non-atomic during concurrent lane work. No further builds or parity captures follow this audit.

### Run 67: raw-target SVG paint and link admission (2026-10-25)

Read the complete brief through **Update 30**, including its initial Update,
newest reports/messages and STATUS. Selected eight d2svg functions as one
shared admission cluster, beyond compiler-produced board coverage. The original
Go tests retain every assertion; the observer snapshots inputs before rendering.
Native results compare complete errors, nil outputs and successful SVG bytes,
without normalization. The Go test binary uses the normal v0.9.0 release stamp.

Static source comparison found missing **d2svg/paint_validation.go:15–161**,
ordinary link checks (**d2svg.go:2866–2900**) and RenderLegend's link check
(**259–267**). Rust now checks root/shape/connection/legend paints and fill
patterns, both palette override sets, and encoded dangerous links in upstream
order. It reuses native gradient parsing, csscolorparser and link decoding.
ThemeCSS also rejects invalid override sets at its direct boundary.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Raw-target calls, eight original functions | **19/60 → 60/60** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| cli-diff, explicit shared log/PDF clock | **119/119 → unchanged** | **0** |
| Strict shared-clock/release CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |

All **41 rejection failures** are fixed; all **19 supported outputs** retain
identical bytes, including mixed-case fill patterns and valid palette/gradient
paints. All **966 e2e outcomes**, 119 cli-diff rows, 70 CLI cases and 66 golden
classifications are unchanged. One upstream skip and two injected writers stay
non-passing. This adds source-call evidence, not golden-board matches or full
structural/cyclic-target validation. Other scoreboards are inherited.

**265 native render/example tests + seven Python harness tests pass**. All 265
repeat from compressed warm executables; compressed API replay remains 60/60,
and complete CLI capture rescoring remains 67/70 / 51. The original Go test
files are restored and the helper removed. Scoped formatting, Ruff and diff
checks pass. Capture limits: **1 GiB total / 128 MiB file / 20 GiB free**, with
streaming guards and finite deadlines. Shared Cargo lock waits are recorded.

Evidence: `.reference/codex-runs/run67-{deltas,source}.json`,
`run67-svg-capture`, negative/final/compressed native results, fresh e2e/CLI
reports and test logs. `run67-verify.py` verifies **447 retained bindings**,
all original/native inputs and results, fixture output hashes, restored Go
sources and the retired build variant's replacement. Frozen API runner:
`target/diagnostic/examples/svg-admission-run67-final`; CLI:
`target/diagnostic/d2-rs-run67-final`. New alias: `svg-render-admission.json`.
No complete source freeze of concurrent other-lane CLI/renderer edits is claimed.

Fresh whole-repository inventory: **673 added / 198 named observations / 475
without**, including other lanes' completed work. Lane F **141 → 133** without
named observations: **CLI 65, d2svg 20, layouts 31, WASM/JS 8, PNG/report 9**.
No applicability exclusions were added. Remaining raw-target structural
admission, font registration and isometric dispatch tests need separate capture;
these coverage gaps are not automatically native failures. No commits.
Cleanup removed two superseded pre-fix Cargo outputs (**10.48 MiB allocated
path footprint**), preserving the identical negative runner and all requests/
results. Verified Applesauce 0.5.30 / LZFSE saves **108.47 MiB across 16
operations**, with hashes/modes/owners/original xattrs preserved and clone
backups removed. Ledgers/rollback: `run67-cleanup.json`,
`run67-{before,final}-compression.json`. Savings are path/operation figures,
not concurrent net physical-volume savings. No bulk moved.

Final allocated/apparent GiB: checkout **22.27/36.82**, `.reference` **11.08/22.60**, `target` **9.73/12.40**; **103.00 GiB free**. Every ceiling passes (`run67-disk-final.json`, raw du/df); both full scans exit zero without errors, include hidden paths/.git and count every apparent hardlinked path. Measurements are non-atomic during concurrent lane work. No further builds or parity captures follow this audit.

### Run 66: formatter warning and executable/API exit errors (2026-10-25)

Read the entire brief through **Update 29**, including its initial Update,
latest reports/messages and STATUS. Lane F's remaining reproducible CLI
failure was **`fmt --check uf.d2`**. Static comparison of **d2cli/fmt.go:18–69**,
**lib/log/prettyhandler.go:30–51** and **util-go v0.2.0 xmain/xmain.go:58–82**
found two boundaries: warnings belong on process stdout, with ANSI colors
and local RFC3339 time; mainFatal extracts ExitError.Message while the API
retains the deferred formatter context. Native typed command errors preserve
both. Other error contexts are unchanged.

| Tier | Before → final | Regressions |
|---|---:|---:|
| cli-diff, explicit shared log/PDF clock | **118/119 → 119/119** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Strict release/shared-clock CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |

The clock tier compares complete output bytes at UTC **946782245**;
`scripts/build-cli-clock-oracle.py` installs clock helpers only in the pinned
trace checkout, builds a release-stamped oracle, then restores touched files.
`cli-diff-scoreboard.py --go <clock-oracle> --epoch 946782245` supplies the
same clock to Rust. No timestamp stripping or artifact normalization is added.
Ordinary before/after captures each have 118 matches, with different temporal
rows failing; ordinary wall-clock byte identity is not claimed.

The subprocess adapter now accepts a native structured error record and a
separate process-log sink. It validates the exit code and terminal message,
returns the actual native API error, and forwards process logs unchanged.
All original Go assertions remain active. An intermediate build failed the
in-process formatter assertion; a subsequent adapter pilot shadowed its Go
error variable. Both remain negative evidence; final CLI delta is zero.
The first clock pilot's two help failures were a basename setup bug, corrected
before the qualified before/after runs.

**237 native CLI + 20 Python harness + three Go adapter tests pass**;
all 237 native tests repeat from the compressed executable, and the compressed
clock-profile runners repeat 119/119. The complete CLI capture rescores at
67/70 artifacts / 51 goldens. The inherited adapter unit stimulus was corrected
to prefix every multiline terminal-error line, as mainFatal does.
Full row inventories and
zero-regression evidence: `.reference/codex-runs/run66-deltas.json`,
`run66-e2e-{before,sealed}.json`, `run66-cli-clock-{before-qualified,sealed}.json`,
`run66-cli-complete.json` and its complete original capture. Frozen native
runner: `target/diagnostic/d2-rs-run66-final`; new clock-profile alias:
`.reference/scoreboard-master-6a7b4837/cli-diff-clock.json`.
Other language, raster, flag,
watch and isometric goldens are inherited. No commits; other lanes and `29`
are preserved. Final cleanup/disk evidence and next queue: `report-run66.md`.

Cleanup removed the interim runner (**59.87 MiB allocated path footprint**);
verified Applesauce 0.5.30 / LZFSE saves **90.68 MiB over seven idle files**,
with bytes, modes, ownership and original xattrs preserved and clone backups
removed. Exact ledger/rollback: `run66-cleanup.json`,
`run66-{oracle,final}-compression.json`. A received target sample was 12.38 MiB
over its ceiling; idle compression and a fresh whole audit cleared every
ceiling before replays. Final allocated/apparent GiB: checkout
**22.37/37.00**, `.reference` **11.03/22.55**, `target` **9.88/12.51**;
**101.09 GiB free** (`run66-disk-final.json`). The first final scan
hit a concurrent deletion and is non-passing; the repeat exits zero without
errors. Path/operation savings are not net physical-volume savings. No bulk
moved or further builds/captures after the final audit.

### Lane L24: netpolicy client ownership; full master-test inventory (2026-10-25)

Completed Update 29's remaining Lane L cluster: four original netpolicy client
functions, with all Go assertions active. Native client/transport ownership in
`d2-rs-scenebuild/src/netpolicy_client.rs` follows **lib/netpolicy/netpolicy.go:
48–125**. Public-only clients discard caller callbacks and own their DNS,
proxy and redirect boundary; private opt-in preserves shared custom transport.
Native real-socket replay matches **4/4 functions / 10 operations**, covering
complete bodies, endpoint inventories, DNS/numeric addresses, callback use and
ownership. Cancellation interrupts active requests, and mixed DNS answers fail
before dialing. Rust uses ClientBuilder callbacks; arbitrary Go configuration
cloning, Go transport-field identity and connection-pool identity are not claimed.
Error-kind comparisons follow upstream errors.Is, not string normalization.

Before: **four unsupported native APIs**. Final: **four matches**, zero
regressions. Fresh before/final e2e guards remain **933 SVG / 11 expected errors**,
all **966 row outcomes unchanged**. No new golden-board improvement is claimed.
**32 native + 18 harness tests pass**; all 32 repeat from compressed warm test
executables and the compressed client runner repeats 4/4. Other tiers are
inherited. Original Go tests restored, hook removed; finite deadlines and
**1 GiB total / 128 MiB file / 20 GiB free** capture guards retained. Scoped
formatting/lints/diff checks pass. No commits initiated by L24; supervising
0b77fcf3 included the initial adapter during this run. Other lanes' work remains.

Original inventory: **143 / 138 captured / five uncovered** at audit time,
counting I21's two finished functions. **Lane L netpolicy: four → zero**; all
eight functions are covered across L23/L24. Lane I retains the four imageasset
local-policy functions and TestHTTPNetworkPolicy; in-flight work is not credited.

Update 29's whole-repository re-inventory is now recorded by
`scripts/master-added-test-inventory.py`: **673 master-added Go Test functions,
172 with named native observations, 501 without**. This larger denominator
includes scopes omitted from the original 143 and is an observation inventory,
not 501 demonstrated native failures. Largest unnamed groups: isometric image
internals 308, CLI 65, d2svg 28, isometric scene 17, d2lib 10, layoutguard eight.
The earlier instruction to stop the isometric API sweep still applies.
Six added d2js/WASM functions lack named observations (admission, animation
padding/appendix, dangerous legend links, and isometric compile/format handling),
as do two npm-auth CI tests. **d2lsp: ten existing / zero master-added functions**.
Full names, lines and evidence references: `lang24-whole-repo-inventory.json`.
No work outside Lane L's assigned queue was selected.

Evidence: `lang24-{deltas,inventory,source,corpus-check}.json`, source patch,
qualified original capture, negative/final replays and fresh e2e reports.
`lang24-verify.py` checks **17 new input/output bindings**, nine source files,
both retained runners and original assertions; prior **423 L23 + 476 L22 +
475 L21** bindings verify too. New alias: `netpolicy-client-api.json`.
Frozen CLI/external runner: `d2-rs-lang24-qualified` and
`examples/netpolicy-client-scoreboard-lang24-qualified`. The CLI guard predates
later concurrent Lane F edits; its hash is retained, not an exact source freeze.
Handoff: `report-lang24.md`.

Cleanup removes **14 files / 60 KiB allocated path footprint** of superseded
pilots and disposable inspection output. Verified Applesauce **0.5.30 / LZFSE**
saves **50.52 MiB on nine idle files**, preserving hashes/modes/owners/xattrs;
verified clone backups removed. Exact paths and rollback commands are in
`lang24-{cleanup,compression}.json`; no bulk moved or concurrent net physical
volume savings claimed.

Final allocated/apparent GiB: checkout **22.45/37.00**, `.reference` **11.02/22.54**, `target` **9.91/12.82**; **101.09 GiB free**. Every ceiling passes (`lang24-disk.json`, raw du/df), including hidden paths/.git and every apparent hard-linked path. Concurrent lane measurements are non-atomic. No further builds or parity captures follow this audit.

### Lane L23: shared netpolicy address and dial APIs (2026-10-25)

Continued the full brief through arriving **Update 29**: Lane L now owns
netpolicy only; the seven imageasset functions move to Lane I. Captured four
original functions, keeping every Go assertion: TestNonPublicAddressRanges
and all three TestValidatedDial functions. **29 operations** compare complete
errors, lookup arguments and numeric dial histories, including repeated DNS
resolution in one native process.

`d2_rs_scenebuild::netpolicy` translates **lib/netpolicy/netpolicy.go:129–244**:
complete-answer validation, IPv4 unmapping, ordered deduplication, numeric
dialing, joined failures and cancellation. Both production HTTP adapters now
share its resolver/builder. The preserved old predicate matches the range
function; three missing native dial APIs become matches. This is added API
coverage, not a new golden-board fix. The split host/port native API does not
qualify Go's malformed-destination parser or full client/transport API.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New netpolicy functions / operations | **1 match + 3 unsupported → 4 / 29 exact** | **0** |
| Prior network / localfile functions | **4 / 8 → unchanged** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Shared-clock/release CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Fresh cli-diff | **117/119 → 118/119** | **0** |

The ordinary-clock PDF stdout row happens to match; no stable policy fix is
attributed to that incidental result. Formatter logging remains non-passing.
**263 native + 17 harness tests pass**. Compressed runners repeat netpolicy,
network and localfile results; complete strict CLI inventories/artifacts
rescore unchanged. Scoped formatting/lints/diff checks pass. The original Go
tests are restored, hook removed; streaming limits remain **1 GiB total /
128 MiB file / 20 GiB free**, with finite deadlines. Other language, full
raster, flag and isometric tiers are inherited. An overly broad formatting
command was corrected without retaining unrelated formatting or changing
another lane's semantic edits. No commits.

Inventory: **143 functions, 132 captured, 11 uncovered**. **Lane L: eight →
four netpolicy functions**; seven imageasset functions belong to Lane I.
Next L batch: every-redirect checks, caller-dial-hook isolation, bypass-hook
clearing and private opt-in custom-transport ownership. Update 29's whole-repo
master-test re-inventory follows these functions; it has not been done here.

Evidence: `lang23-{deltas,inventory,source,corpus-check}.json`, compact source
patch, original `lang23-netpolicy-final-capture`, negative baseline, qualified
replays and guards. **423 current + 476 L22 + 475 L21 input/output bindings
verify**. New alias: `netpolicy-address-dial-api.json`; network/localfile
aliases advance with prior bindings recorded. Frozen CLI and two examples:
`target/diagnostic/d2-rs-lang23-qualified`, examples
`{netpolicy,asset-policy}-scoreboard-lang23-qualified`.

`lang23-cleanup.json` records **18 files / 29.32 MiB allocated path footprint
removed**. Two superseded L22 runners retire explicitly; all twelve original
L22 source hashes reconstruct from **83895fa0**, and prior input/output
closure survives. Verified Applesauce **0.5.30 / LZFSE** saves **35.16 MiB on
nine idle files**, preserving bytes, executability, ownership and original
xattrs; clone backups are removed. Exact rollback: `lang23-compression.json`.
No bulk moved or concurrent net physical-volume savings claimed.
Handoff: `report-lang23.md`.

Final allocated/apparent GiB: checkout **22.17/36.61**, `.reference` **10.98/22.50**, `target` **9.73/12.53**; **104.14 GiB free**. Every ceiling passes (`lang23-disk.json`, raw du/df), including hidden paths/.git and every apparent hardlinked path. Concurrent lane scans are non-atomic. No further builds or parity captures follow this audit.

### Lane I iso22: imageasset rooted default directory (2026-10-25)

Continued the full brief through **Update 29**, newest completed lane reports
and STATUS. Captured all four original `lib/imageasset/local_policy_test.go`
functions with all Go assertions intact: **28 operations** over actual files,
rooted/denied policies and one shared cache. The native replay receives only
operations and original fixture bytes; expected resources/errors are separate.

Fixed **imageasset.go:353–367**: when BaseDir is omitted, New takes the
local-file policy root before falling back to the process directory. Added
`ImageFileReader::root_path`, implemented by `ImageLocalFiles`; the existing
constructor treats an empty Path as omitted. Explicit `.` and other explicit
directories retain precedence. Before, relative rooted reads failed at
classification and the fetched-byte/symlink calls reached the wrong paths.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Four local-policy observation functions / 28 operations | **2 matches + 2 mismatches → 4 matches** | **0** |
| Prior lifecycle functions / operations | **2 / 15 → unchanged** | **0** |
| Fresh e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

The e2e gate preserves all **966 outcomes**; no new golden is claimed.
The observation tier compares complete resource bytes/metadata, error text,
unavailable flags and cache entries/bytes. Native `ErrDenied` and `LimitError`
typed identities remain **unqualified**: the current public ImageResolveError
carries text and unavailable. Original Go typed-error assertions remain active;
these observations do not imply every Go error API is implemented in Rust.
**39 native + 8 harness tests pass**, including a new omitted/explicit-base
regression. Scoped Ruff, Go formatting and diff checks pass. Unrelated Rust
formatting was restored. Other tiers are inherited; no commits initiated.

Evidence: `iso22-policy-capture`, before/final native requests/results,
`iso22-{deltas,source,inventory,corpus-check}.json`, compact source patch,
e2e guards, tests and original-source restoration. Guards retain **1 GiB total /
128 MiB file / 20 GiB free** streaming limits and finite deadlines. Initial
non-exhaustive replay match and TempDir-parent setup failures were corrected;
their discarded pilots are not passing evidence. Before/final production
comparisons use the same final Go capture and identical native requests.

**Original Lane I queue: 5 → 1**, with `TestHTTPNetworkPolicy` next. L24
closes netpolicy separately, bringing the original denominator to **143 / 142
captured / 1 uncovered**. Arriving **Update 30** supersedes the earlier sweep
stop and assigns Lane I isometric image/scene tests plus remaining imageasset.
A fresh `master-added-test-inventory.py` run with I22 evidence reports **673
added / 176 named observations / 497 without**. **Lane I: 326 without named
observations**: image **308**, scene **17**, imageasset **1**. These are coverage
gaps, not demonstrated failures; no new applicability exclusions or native
scope is selected here. Full names/source lines and d2js/d2lsp audit remain
in `iso22-whole-repo-inventory.json`. Handoff: `report-iso22.md`.

Cleanup removed **17 files / 6.93 MiB allocated path footprint**; verified LZFSE saves **60.22 MiB on 12 currently retained files**. Exact deletions, retirements, targets and rollback commands are recorded in the I22 ledgers; verified clone backups are removed. A brief target overage from the CLI snapshot was cleaned up; a fresh whole-checkout audit passed before final native qualification. These are path/operation figures, not concurrent physical-volume savings.

Final allocated/apparent GiB: checkout **22.37/37.00**, `.reference` **11.03/22.54**, `target` **9.88/12.82**; **101.09 GiB free**. Every ceiling passes (`iso22-disk.json`, raw du/df), including hidden paths/.git and every apparent hardlinked path. Measurements are non-atomic during concurrent lane work. No further builds or parity captures follow this audit.

### Lane I iso21: imageasset resource and transport lifetime (2026-10-25)

Update **29** supersedes the exhausted I20 scope and moves seven imageasset
functions to Lane I. Captured two original functions with all Go assertions
intact: `TestResourceDataURIContext` and
`TestResolverClosesOnlyOwnedHTTPTransport`, **15 observed operations**.
Data-URI encoding already matched; added the missing native resolver transport
ownership and explicit idle cleanup from **imageasset.go:327–419**.
Borrowed clients remain caller-owned; boxed clients explicitly transfer cleanup
ownership. The default native HTTP client retains no idle connection pool.
The finite empty-standard-transport witness uses an independent native default
client; arbitrary configuration cloning/active-connection behavior remains
unqualified. Rust absence uses Option instead of nil receivers.

Lifecycle: **1 match + 1 unsupported → 2/2**, **zero regressions**. Fresh
rebuilt CLI e2e: **933 SVG matches / 11 expected errors**; all **966 outcomes
unchanged**. **35 native + 6 harness tests pass**, including prior imageasset
resource/cache/HTTP fixtures; 27 native tests and the complete lifecycle/e2e
guards repeat using verified compressed executables. Other tiers are inherited.
Capture guards remain **1 GiB total / 128 MiB file / 20 GiB free**, with finite
deadlines; original trace tests are restored. No golden gain or commits.

Evidence: `iso21-{deltas,source,inventory,corpus-check}.json`, source patch,
`iso21-lifecycle-qualified-capture`, corrected negative/complete-final native
requests and outputs. The verifier checks 12 capture/input/output bindings,
seven source files and runner hashes/retirement. Alias:
`imageasset-lifecycle-api.json`. Initial mutable-slice observer, wrapper-cwd and
lint setup failures are corrected and not counted as passing evidence.
Handoff: `report-iso21.md`.

**Lane I uncovered: 7 → 5**. Next: the four local-policy functions, then
`TestHTTPNetworkPolicy`. Including L23: inherited inventory **143 functions,
134 captured / 9 uncovered** (imageasset five; netpolicy four).
Whole-repository re-inventory remains queued after both assigned queues,
per Update 29. No new inventory coverage is inferred.

Cleanup removed **22 files / 2.52 MiB allocated path footprint**: pilots, superseded/repeated outputs and the obsolete negative executable. Corrected negative inputs/outputs and its source/rebuild record survive. Verified Applesauce **0.5.30 / LZFSE** saves **47.36 MiB across 12 idle files**, preserving hashes, modes, owners and original xattrs; clone backups removed. Exact targets/rollback: `iso21-compression.json`; deletion ledger: `iso21-cleanup.json`. These are path/operation figures, not net physical-volume savings. No bulk moved. APFS/local mount verified; DiskManagement inspection was unavailable.

Final allocated/apparent GiB: checkout **22.24/36.67**, `.reference` **10.98/22.50**, `target` **9.74/12.48**; **104.90 GiB free**. Every ceiling passes (`iso21-disk.json`, raw du/df), including hidden paths/.git and every apparent hardlinked path. Measurements are non-atomic during concurrent lane work. No further builds or parity captures follow this audit.

### Lane I iso20: assigned queue exhausted; regression guard (2026-10-25)

Re-read the complete brief, including the initial Update and **Update 28**,
and newest I19/L22/F64 reports/messages. Update 28 supersedes Update 27;
its SVG API work and raw-golden audit are already complete in I19. The
earlier color and named-subtree stdout items are complete in I18/I17.
**No uncompleted Lane I item or qualifying failing board remains in this
scope.** GIF/timeline and asset/policy work retain their existing lane owners.
No production edits, builds, new API sweeps, oracle instrumentation or commits.

Fresh e2e guard, using the existing frozen **L22 qualified CLI**:
**933 SVG matches / 11 expected errors**, all **966 row outcomes unchanged**,
**zero regressions** against the master baseline. The other 22 rows remain
explicitly inapplicable or without goldens. Streaming limits: **1 GiB total,
128 MiB/file, 20 GiB free**, 120-second board / 900-second run deadlines.

Verified retained I19 source/capture hashes, original assertion events,
**51 SVG calls / 11 functions**, **204 native input/output hashes**, and all
**13 raw-golden audit rows**, including input/golden/output and oracle/runner
bindings. Current aliases remain **51/51 SVG**, **16/16 color**, and
**72/72 target links**; those calls were not executed again. Raw SVG remains
**939/952**, with **11 stale in pinned Go and two decode errors**; no new
golden gain is claimed. Other scoreboards and native test counts are inherited.
Lane I uncovered SVG/color functions remain **0 / 0**.

Evidence: `.reference/codex-runs/iso20-{queue-check,e2e,e2e-run,deltas}.json`,
`iso20-e2e.log`; cleanup/compression and final disk measurements are recorded
in `iso20-{cleanup,compression,disk}.json` and the raw du/df files. Only empty
run directories and the verified compression backup were removed; no historical
bulk was retired. Verified Applesauce **0.5.30 / LZFSE** reduces the retained
e2e report by **144 KiB**, preserving bytes, mode, ownership and original
xattrs. This is not a physical-volume or concurrent net-savings claim.
Handoff: `report-iso20.md`. Await a new scoped assignment; do not restart the
closed isometric API sweep or take another lane's failures.

Final allocated/apparent GiB: checkout **22.21/36.64**, `.reference`
**10.97/22.49**, `target` **9.78/12.46**; **103.00 GiB free**. Every ceiling
passes; measurements include hidden paths/.git and every apparent hardlinked
path. Concurrent lane activity makes the scans non-atomic. No further builds
or parity captures follow this audit.

### Run 65: isometric GIF CLI output (2026-10-25)

Continued Lane F after reading the full brief through **Update 28**, including
its initial Update, newest reports/messages and STATUS. Pin **6a7b48378**.
`main.rs` now passes the signed animation interval into `isometric_cli.rs`.
Following **d2cli/isometric_image.go:120–145**, a leaf uses native Render;
subboards or an explicit interval use RenderTimeline, with a default 1000 ms
hold. Folder-only boards add no blank frames. GIF writes one atomic file or
stdout, retaining caller cancellation and source/flag mode selection.

The first complete CLI comparison exposed four single-frame timeline loop
mismatches. **Go 1.27 image/gif/writer.go:173** emits NETSCAPE2.0 only for
multiple frames; native timeline encoding now follows that condition.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New isometric GIF CLI cases | **3 errors + 13 render failures → 16/16** | **0** |
| Complete new GIFs / composited frames | **0 → 13 / 424** | **0** |
| Combined isometric static/paged/GIF CLI | **74 existing passes → 90/90** | **0** |
| Fresh e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Shared-clock/release CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |

The finite CLI witnesses follow `isometric_test.go:126–149` and
`isometric_image_test.go:89–165,216–244`; they do not execute all in-process
Go assertions or claim new goldens. The existing GIF gate compares every
composited RGBA frame, canvas, delay and loop. Source/flag GIF bytes also
match within each implementation. Encoded Go/Rust GIF byte identity is not
claimed. **255 native tests + 47 harness tests pass**, including the new
one-frame hold/loop regression, all 235 CLI tests, prior GIF/timeline calls
and all 38 raster-harness tests. No shared raster harness was changed.

Evidence: `run65-{deltas,corpus-check}.json`, complete before/final GIF
captures, `run65-isometric-cli-qualified.json`, fresh e2e and strict CLI
reports and test logs under `.reference/codex-runs/`. Fresh aliases:
`isometric-cli-gif.json`, `isometric-cli-all.json`. Qualified runner:
`target/diagnostic/d2-rs-run65-qualified`. Captures retain **1 GiB total /
128 MiB per file / 20 GiB free** streaming guards and finite deadlines.
Other language, full raster, isometric SVG/PNG goldens, flags and watch tiers
are inherited. An initial guard invocation used a relative script path from
the trace cwd; the corrected absolute-path run passes. Intermediate loop
mismatches remain negative evidence; neither attempt counts as a final pass.

Lane F's assigned GIF/timeline queue is closed for these measured cases.
Performance/static raster caching, SVG-import ownership and custom-font
registry remain separate, unqualified work. Select further changes only
from a named failing case; the existing cli-diff formatter/PDF-stdout rows
need source/profile confirmation before investigation. No commits; other
lanes and `29` remain intact. Cleanup and final disk figures are recorded in
`report-run65.md`.

Final allocated/apparent GiB: checkout **22.17/36.61**, `.reference` **10.98/22.50**, `target` **9.73/12.47**; **104.52 GiB free**. All ceilings pass
(`run65-disk.json`; both du calls exit zero). `run65-cleanup.json` records
**481 files / 63.37 MiB** removed; verified LZFSE saves **104.91 MiB on
14 files**, with backups removed and hashes/modes/owners/xattrs preserved.
These are path/operation figures, not net physical-volume savings. No bulk
moved or further builds/captures after the final audit.

### Lane I iso19: typed SVG contracts and raw-golden audit (2026-10-25)

Continued the full brief through **Update 28**, including the initial Update,
latest I18/L21/F63 handoffs, STATUS and the scoreboard workflow. Stayed in
Lane I's SVG scope; no GIF, timeline, imageasset or imgbundler edits, commits
or changes to `29`.

Added native `d2_rs_render::svg` attributes, ordered serialization, parsing,
local IDs/IRIs and text/trusted fragments, mapped to **lib/svg/markup.go:15–324
and text.go:11–53** at master **6a7b48378**. Fields remain private; malformed
and event-handler names, duplicates, markup fragments, unsafe IRIs and invalid
trusted XML values fail. Existing renderer scoped IDs use the same constructor;
text escaping reuses the existing renderer implementation. The Go observer
keeps every original assertion and preserves raw attribute bytes and nil
slices. Raw Go errors survive; native results compare the original error/panic
boundary, without claiming exact decoder error text.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Eleven newly covered SVG functions / 51 calls | **6 existing + 45 unsupported → 51/51** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Raw SVG goldens | **939/952 → unchanged** | **0** |
| Pinned-Go audit of 13 non-passing raw rows | **11 stale + 2 decode errors** | No golden gain |

The negative runner uses the exact old private `svgtext` source; only scoped
IDs and UTF-8 text were available. Its 51 native requests/outputs survive.
The baseline outputs were rebound to the final byte-faithful Go capture after
verifying all requests are identical; no second baseline execution is claimed.

All eleven renderable stale rows also match pinned Go. Stale groups:
`measured/empty-markdown` (dagre), `regression/empty_md_measurement`,
`stable/tooltips`, `txtar/c4-theme`, `txtar/dark-theme-shape`, and
`txtar/sequence-bounding-box#01` (last five: dagre/elk). Pinned Go rejects both
`stable/3d_sketch_mode` target JSONs because `srcLabel` is a string instead of
`d2target.Text`. None of these thirteen rows becomes a golden pass. The new
master adapter replaces use of the old v0.9.0 standalone oracle for this audit.
No new normalization, exclusions, tolerances or fixture-specific runtime paths.

**267 native + 11 harness tests pass.** The native suite includes the 51 SVG
calls and 27 prior color witnesses. Fresh e2e and full raw golden runs pass
their existing gates; every audited output is inventoried and nonempty SVGs
are XML-validated. Source tests are restored. Captures retain streaming
**1 GiB total / 128 MiB file / 20 GiB free** guards and finite deadlines.
Other language, raster, document and isometric tiers are inherited. Initial
setup, panic-log transport, xattr inspection and overly broad build-guard
failures were corrected and are not passing evidence. Final transport retains
Go nil versus empty slices; the earlier coalesced pilot is retired.

Evidence: `iso19-svg-byte-{before,final}.json`, original
`iso19-svg-byte-capture`, `iso19-raw-svg-qualified-audit{,.json}`,
`iso19-{deltas,source,corpus-check}.json`, compact source patch and test/build
logs. Aliases: `svg-api.json`, `svg-raw-golden-audit.json`.
`iso19-verify.py` verifies **204 native input/output files**, all 51 original
calls and all thirteen raw input/golden/output bindings. Exact old source and
adapter remain; the obsolete negative executable is explicitly retired.

**Lane I uncovered SVG functions: 11 → 0.** Update 28's raw-golden audit is
also closed without a qualifying golden fix. No further item remains in the
current Lane I queue; GIF/timeline and the remaining asset/policy work stay
with their assigned lanes. These finite calls do not establish full SVG API
or fuzz coverage.

Cleanup removed **635 files / 166.62 MiB allocated path footprint**,
with replacements/rebuild instructions in `iso19-cleanup.json`. Current inputs,
expected outputs and negative native witnesses verify. Verified Applesauce
**0.5.30 / LZFSE** saves **163.81 MiB across 30 operations**;
hashes, modes, owners and original xattrs verify, with clone backups removed.
`iso19-compression-retirement.json` identifies three later-superseded executable
bindings; operation-level reductions are not physical-volume or net checkout
savings. Exact targets/rollback: `iso19-compression.json`. No bulk moved.

Final allocated/apparent GiB: checkout **22.08/36.51**, `.reference`
**10.95/22.47**, `target` **9.67/12.40**; **106.43 GiB free**.
Every ceiling passes (`iso19-disk.json`, fresh raw du/df). Hidden paths/.git
and every apparent hardlinked path are included. No further builds or parity
captures follow this audit. Handoff: `report-iso19.md`.


### Run 64: native ordered-board GIF timelines (2026-10-25)

Continued Lane F's GIF queue at **6a7b48378**, after the full brief through
**Update 28** and latest handoffs. `img/timeline.rs` translates
**timeline.go:20–190** and **framing.go:253–270**: measure each board's final
geometry into one camera, sample a shared palette, then encode board order,
authored motion and cumulative delays without changing source diagrams.
The shared camera stays internal; capture sessions share the outer deadline.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original timeline calls | **12 unsupported → 12/12** | **0** |
| Complete timeline GIFs / composited frames | **0 → 3 / 10** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Isometric static/paged CLI | **74/74 → unchanged** | **0** |
| Shared-clock/release CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |

Four original Go test functions retain every assertion. The GIF gate checks
every composited RGBA frame, canvas, delay and loop; native tests also bind
palette hashes and delta rectangles. **19 native + 8 harness tests pass**,
including the existing 592 GIF frames and 32 PNG frames. All native tests
repeat from compressed executables. Original source/output closure verifies
after cleanup, including 128 isometric CLI output hashes and the prior GIF/PNG
bindings. No encoded GIF byte-identity or new golden claim. Full language,
ordinary raster, isometric SVG/PNG goldens, flags and watch tiers are inherited.

Evidence: `.reference/codex-runs/run64-{deltas,source,corpus-check}.json`,
`run64-timeline-final-capture/`, `run64-timeline-final.json`, native/harness
logs and the retained unsupported baseline. New alias:
`.reference/scoreboard-master-6a7b4837/isometric-timeline.json`.
Frozen runners: `target/diagnostic/d2-rs-run64-final` and
`target/diagnostic/examples/isometric-timeline-run64-final`.
Captures retain **1 GiB total / 128 MiB per file / 20 GiB free** guards.
An initial isometric PDF guard used the ordinary clock oracle; its corrected
run uses the established shared-clock profile and passes all 74 cases.

**Next:** CLI GIF wiring in `isometric_cli.rs` and its `main.rs` interval
handoff, following **d2cli/isometric_image.go:120–145**. Target
`TestIsometricSourceGIFUsesSameAnimationAsFlag`,
`TestIsometricImageMultiboardExports` and folder-only GIF traversal. The native
single-board GIF and RenderTimeline APIs are ready; CLI GIF remains absent.
Static raster caching/performance remains unqualified. No commits; other lanes
and `29` are preserved. Final cleanup/disk evidence is in `report-run64.md`.

Cleanup removed **1,392 files / 165.77 MiB allocated path footprint**; verified
LZFSE saves **89.55 MiB across nine operations**, including a retired interim
runner. Backups are removed; original hashes/modes/owners/xattrs verify.
`run64-cleanup.json`, `run64-runner-retirement.json` and
`run64-compression.json` record replacements and rollback commands. These
figures are not additive physical-volume savings; no bulk moved.

Final allocated/apparent GiB: checkout **22.16/36.56**, `.reference` **10.95/22.47**, `target` **9.75/12.45**; **106.81 GiB free**. Every ceiling passes (`run64-disk.json`, raw du/df), including hidden paths/.git and every apparent hardlinked path. No further builds or parity captures follow this audit.

### Lane L22: remaining imgbundler and standalone localfile policies (2026-10-25)

Continued the full brief through **Update 28**, newest reports/messages and
status. Stayed in Lane L scope; no main.rs, isometric, SVG or color edits.
No commits. Captured **nine original master-added functions**: the remaining
`TestBundleRemoteNetworkPolicy`, followed by all eight localfile functions.
All original Go assertions run, including both FIFO subtests.

`imgbundler::bundle_remote_with_policy` maps **lib/imgbundler/imgbundler.go:
121–180**, reusing native HTTP policy, per-invocation resolver/cache ownership
and common legacy limits. `ImageLocalFiles::{open,policy_cache_key,try_close}`
maps **lib/localfile/localfile.go:101–201**, sharing the retained-descriptor
walk with image reads. Lexical denials preserve typed ErrDenied identity;
copies share one actual close and its result. Caller-owned open descriptors
remain readable after root closure. All algorithms remain native Rust.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New remote bundling calls | **4 unsupported → 4 exact** | **0** |
| New standalone localfile functions / operations | **8 unsupported → 8 / 104 matched** | **0** |
| Existing local-policy calls | **8 exact → unchanged** | **0** |
| Existing resolver calls | **12 exact + 1 assertion → unchanged** | **0** |
| Fresh e2e SVG / errors | **933 / 11 → unchanged** | **0** |
| Shared-clock/release CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Fresh cli-diff | **117/119 → unchanged** | **0** |

Network replays use a real loopback server and compare complete output/error
bytes plus endpoint hit counts, consecutively in one native process. Localfile
replays recreate the actual writes, symlinks, rename and FIFOs, returning real
opened descriptors and replaying reads/closes. The complete cache-key equality
matrix matches the source tests; independent random scope bytes are retained
raw on both sides, without claiming byte equality. The missing-API baseline
adapter and negative outputs survive. No golden gain or normalization is added.

**268 native + 22 harness tests pass**. Compressed executables repeat 33
native tests; the compressed CLI test executable lists all 235 tests. Compressed
qualified runners repeat the new 12 results, existing local/resolver calls and
the full e2e/CLI guards. Scoped formatting/lint/diff checks pass. One new test
closure-lifetime error and the initial Python test path/lint setup errors were
corrected. Initial attempts are not passing evidence. Trace test sources are
restored and helpers removed. Capture guards remain **1 GiB total / 128 MiB
file / 20 GiB free**, with finite deadlines.

Fresh cli-diff retains `fmt --check uf.d2` logging and PDF stdout as failures.
The old frozen L21 runner and current master baseline also give **117/119**;
the historical L21 report's 118/119 is not reused as current evidence. Other
language, flags, full raster and isometric tiers are inherited.

Inventory after reading the arriving **I19/F64** reports: **143 master-added
functions, 128 captured, 15 uncovered**, including I19's independent eleven
SVG functions. **Lane L: 24 → 15** (imgbundler 0, localfile 0, netpolicy 8,
imageasset 7); Lane I's SVG/color queue is closed. **Next: netpolicy's eight
functions**, then imageasset's seven.
Other-platform rooted APIs, non-witness OS/close errors, and the existing
automatic five-minute bundler timer/logger remain unqualified.

Evidence: `lang22-{deltas,inventory,source,corpus-check}.json`, compact source
patch, original `lang22-{network,localfile}-capture/`, negative baselines, final
replays and test logs under `.reference/codex-runs/`. **476 current and 475
prior input/output bindings verify**. New aliases: `imgbundler-network-policy-api.json`
and `localfile-api.json`; existing local-policy/resolver aliases advance to L22.
Frozen runners: `target/diagnostic/d2-rs-lang22-qualified` and examples
`{asset-policy,imgbundler-policy,imgbundler-resolver}-scoreboard-lang22-qualified`.

`lang22-cleanup.json` records actual scoped deletions; `lang22-retirement.json`
retires three superseded L21 runners, with exact prior source reconstruction
from **04ea7f97**, original hashes, rebuild commands and new qualified runners.
Original inputs/expected outputs and negative witnesses remain. Verified
Applesauce **0.5.30 / LZFSE** preserves checksums, modes, owners and original
xattrs; clone backups are removed. Details and rollback commands are in
`lang22-{tests-,}compression.json`. **15 files / 134.73 MiB allocated path
footprint deleted**; verified compression saves **87.30 MiB** on 11 idle files.
No bulk moved or concurrent physical-volume/net savings attributed.

Final allocated/apparent GiB: checkout **22.16/36.56**, `.reference`
**10.95/22.47**, `target` **9.67/12.40**; **107.57 GiB free**. Every ceiling
passes (`lang22-disk.json`, raw du/df), including hidden paths/.git and every
apparent hardlinked path. No further builds or parity captures follow this audit.

### Lane I iso18: master gradient admission and escaped attributes (2026-10-25)

Continued **Update 27**, after the latest handoffs and the full brief including
its initial Update. The six stdout failures were already closed by iso17.
Targeted `TestParseGradientValidatesStopPositions` and
`TestGradientToSVGEscapesConstructedAttributes`, with the existing one-stop
test as a guard. No commits; other lanes' files and `29` remain untouched.

`d2-rs-render/src/d2svg/color.rs` now follows **lib/color/gradient.go:105–142,
152–174,238–252**: malformed stops and non-finite/non-numeric positions fail;
positions use canonical shortest Go numbers; IDs, positions and colors use
Go HTML escaping even for caller-constructed gradients. Public native gradient
APIs share this implementation. The new source hook keeps all original Go
assertions active and retains raw errors. Native parsing compares the original
nil/error boundary (`Option`), rather than claiming exact error text; five
ValidColor booleans exercise the existing compiler's quoted-fill consumer.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original color calls (three functions, two master-added) | **9/16 → 16/16** | **0** |
| Additional Go number/radial witnesses | **11/11** | **0** |
| Fresh e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Direct raw golden matches | **939/952 → unchanged** | **0** |

The direct golden run explicitly retains **11 mismatches and two decode
errors** as non-passing; their historical Go classifications were not
requalified here. No new golden gain, normalization, exclusion or tolerance.
Full ordinary raster, CLI document, language and isometric tiers are inherited.

**266 native + 5 harness tests pass**, including all 27 Go API witnesses.
The compressed test executable repeats all 264 library tests; compressed
qualified runners repeat 16 captured calls and the three gradient e2e boards.
Scoped Rust/Python/Go formatting, Ruff and diff checks pass. The first adapter
syntax error, supplemental Go module-path error and compression-inspection
setup error were corrected. A concurrent Lane L cache-signature build failure
passes on retry without edits outside this lane; failed attempts are not
passing evidence. Trace tests are restored. Streaming capture budgets remain
**1 GiB total / 128 MiB file / 20 GiB free**, with finite deadlines.

Evidence: `iso18-color-{before,qualified}.json`, original
`iso18-color-qualified-capture/`, eleven extra witnesses in
`iso18-position-capture/`, `iso18-deltas.json`, source mapping/patch, native
and harness logs, `iso18-corpus-check.json` (**64 request/result bindings**).
New scoreboard alias: `color-api.json`; frozen runners are
`target/diagnostic/d2-rs-iso18-qualified` and
`target/diagnostic/examples/color-scoreboard-iso18-qualified`.

**Lane I uncovered functions: 13 → 11** (SVG 11, color 0). Next capture
`lib/svg/text_test.go` attribute parsing/serialization and scoped-ID contracts;
do not reopen the isometric API sweep, GIF or the six stdout cases.
`iso18-cleanup.json` records **55 obsolete/duplicate files / 0.25 MiB allocated
path footprint** deleted. Verified Applesauce **0.5.30 / LZFSE** saves
**92.25 MiB** on three idle single-link executables, preserving hashes, modes,
ownership and original xattrs; backups are removed. Exact paths and rollback
commands: `iso18-compression.json`.

Final allocated/apparent GiB: checkout **22.12/36.61**, `.reference` **10.92/22.38**, `target` **9.86/12.58**; **107.57 GiB free**. Every ceiling passes (`iso18-disk.json`, raw du/df); scans include hidden paths/.git and every apparent hard-linked path during concurrent activity. No further artifact-heavy work follows this audit.

### Run 63: native GIF image encoding (2026-10-25)

Read the latest reports/messages, current status and the entire brief through
**Update 27**, including its initial Update; also read arriving L20/I18. Continued
Lane F's GIF queue without repeating the closed still/paged/watch work.
No commits; other lanes' changes and `29` remain intact.

`img/gif.rs` follows pinned master **6a7b48378**, **gif.go:20–190**:
four-phase shared median-cut palette, immutable 18-bit RGB lookup, opaque
delta rectangles, cumulative frame delays, infinite looping and bounded
output. The existing Rust go-quantize translation is shared from d2-rs-export.
`gif_loop.rs` follows **gif_loop.go:16–87**, privately copying panel/mask
documents and fitting repeating tracks to the chosen loop. Authored source
timing remains unchanged. `render_with_context` now dispatches all three
image formats after admission (**render.go:99–114**).

The new capture retains the original Render/admission assertions plus five
whole-board witnesses: traffic, bidirectional traffic, authored node, static
route and animated `txtar/legend-mono` legend. The pre-change adapter explicitly
reports the absent GIF API and preserves the existing admission paths.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New GIF image calls | **11 errors + 6 unsupported → 17/17** | **0** |
| Complete GIFs / composited frames | **0 → 6 / 592** | **0** |
| Fresh e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Strict shared-clock/release CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Isometric static/paged CLI | **74/74 → unchanged** | **0** |

GIF comparison uses the existing CLI gate for every composited RGBA frame,
delay, canvas and loop count. Encoded GIF byte identity is not claimed.
**17 native + 8 harness tests pass**; native tests also bind all source palette
hashes/delta rectangles, preserve typed cancellation, and cover private mask
timing, clone admission and atomic writer limits. Eight original Go functions
pass. Compressed executables repeat all native tests, all 17 GIF calls and
two complete CLI SVG/PNG artifacts. Scoped format/lint/diff checks pass.
Other language, full raster, isometric SVG/PNG goldens, flags, links and watch
tiers are inherited. I17 has already closed the six subtree-stdout cases.

Evidence: `run63-{deltas,corpus-check,inherited-closure}.json`, original
`run63-gif-qualified-capture/`, negative baseline, final
`run63-gif-final-qualified.json`, test/build logs and `report-run63.md` under
`.reference/codex-runs/`. The closure verifies 12 complete GIF bindings and
preserves all 64 prior exact-time PNG bindings. New alias: `isometric-gif.json`;
previous aliases are unchanged. Runners are `target/diagnostic/d2-rs-run63-qualified`
and `target/diagnostic/examples/isometric-gif-run63-qualified`. Capture guards
remain **1 GiB total / 128 MiB file / 20 GiB free**. Initial import/lint setup
errors and an in-progress Lane L CLI type error were corrected; they are not
passing evidence.

**Next:** `timeline.go` common camera/frame scheduling, then the original
`TestIsometricSourceGIFUsesSameAnimationAsFlag` and
`TestIsometricImageMultiboardExports` command paths. RenderTimeline and animated
isometric CLI output remain unimplemented. Static rasters are still recomputed
per frame; performance parity, arbitrary API coverage and full D2 compatibility
are not claimed.

Cleanup/retirement: `run63-cleanup.json` and `run63-test-retirement.json`.
Superseded idle run62 tests have passing run63 replacements; their source,
original input/output closure and frozen API runners survive. A transient
**10.005 GiB** target overage was caught immediately after snapshots; new
parity runs stopped, scoped cleanup completed, and the fresh whole-checkout
`run63-disk-restored-*` audit proves every ceiling passes. Final disk numbers
and verified LZFSE savings are recorded in `run63-disk.json`; clone backups
are removed. **55 files / 66.96 MiB allocated path footprint** were deleted;
verified LZFSE saves **77.66 MiB across nine operations**, including one
subsequently retired duplicate. These are not additive physical-volume savings.
Final allocated/apparent GiB: checkout **22.18/36.60**, `.reference`
**10.87/22.38**, `target` **9.78/12.59**; **107.57 GiB free**. Every ceiling
passes; scans include hidden paths/.git and every apparent hard-linked path.
No bulk was moved or concurrent volume savings attributed. No further
artifact-heavy work follows this audit.

### Run 62: exact-time capture for GIF/timeline (2026-10-25)

Implemented the missing shared animated capture prerequisite from Update 26.
The full brief (including its initial Update), latest handoffs and arriving
L19/I15 reports were read. This run makes no commits; concurrent commits,
other lanes' edits and `29` are preserved.

`img/gif_capture.rs` translates **render.go:117–150**, **native.go:24–90**,
**geometry.go:687–728** and **source_panels.go:281–346**. NativeScene retains
packet routes, animated triangle ranges, panel documents and pixel scale.
Frames draw traffic after static depth, keep both shadow cameras fixed, and
sample authored panel tracks at exact nanosecond time. The ARM64 node-motion
FMSUBD follows the retained source/assembly inventory. Source diagrams remain
unchanged; PNGs are caller-owned; callback errors retain their typed source,
and cancellation is checked after the final callback too. Native algorithms
remain Rust.

The new source capture retains four original Go test functions' public
CaptureFrames calls and five named whole-board witnesses: traffic,
bidirectional traffic, animated node, unanimated route, and animated
`txtar/legend-mono` legend. Original assertions stay active. Every emitted
PNG, including prefixes before callback errors/cancellation, passes the shared
raster gates and then complete encoded-byte comparison. No new normalization,
exclusion, tolerance or golden claim is added.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New capture API sessions / complete PNG frames | **0 → 15/15 / 32** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Whole-board isometric SVG / source errors / native PNG | **870 / 138 / 20 → unchanged** | **0** |
| Isometric CLI / watch stages | **74 / 20 → unchanged** | **0** |
| Replayed existing still-image API calls | **234/234 → unchanged** | **0** |
| Release/shared-clock ordinary CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Target links / flag matrix / observed cli-diff | **66/72 / 304/304 / 118/119 → unchanged** | **0** |

The ordinary cli-diff PDF row remains clock-unstable; no PDF improvement is
claimed. Its strict CLI tier retains the established shared UTC epoch,
catalog sort and release stamp. Six named-subtree stdout cases still fail.
I15's additional 30 rich-board calls, language and full ordinary raster tiers
are inherited, rather than newly replayed here.

**11 native + 7 harness tests pass**, including all 32 Go PNG hashes, typed
callback failures, final-callback cancellation, child-deadline isolation,
source immutability and the independent node/packet loop seam. Final native
tests rebuild the formatted source. Compressed executables repeat all eleven
native tests, all fifteen capture sessions and two complete CLI SVG/PNG files.
Scoped rustfmt/Ruff/diff checks pass. Initial missing-runner/module/Pillow
setup failures were corrected and are not counted as passing evidence.

Evidence: `run62-frames-{negative,final}.json`, the original source capture,
all guard reports, `run62-deltas.json`, `run62-corpus-check.json` (**64 exact
PNG bindings**), source hashes/compact patch, and test logs under
`.reference/codex-runs/`. New alias: `isometric-frames.json`; prior aliases
remain intact. Frozen CLI: `target/diagnostic/d2-rs-run62-qualified`.
Capture limits remain **1 GiB total / 128 MiB file / 20 GiB free** with
streaming guards and finite subprocess deadlines. Native fixture:
`tests/fixtures/gif-frames-api.json`.

**Next:** port `gif.go` palette/index/delta/delay handling and `gif_loop.go`
private document timing, then `timeline.go` common camera and CLI GIF output.
The capture currently recomputes static rasters per requested frame; cached
raster/performance parity is not claimed. GIF encoding, RenderTimeline and
CLI GIF remain unimplemented. The existing animated SVG witness, six subtree
stdout cases, SVG-import ownership handoff and custom-font registry remain
open; do not repeat closed still/paged/watch qualification.

Cleanup actually removed **2,315 disposable files / 1,231.34 MiB allocated
path footprint**. `run62-cleanup.json` and `run62-replay-retirement.json`
record every retirement and its retained source/golden/rebuild replacement;
negative witnesses and complete new frame evidence survive. Verified
Applesauce **0.5.30 / LZFSE** saves **90.16 MiB** on nine idle single-link
files, preserving hashes, modes, owners and original xattrs. Clone backups
are removed; exact rollback commands are in `run62-compression.json`.
No bulk was moved, and concurrent volume/net changes are not attributed.

Final allocated/apparent GiB: checkout **21.78/36.07**, `.reference`
**10.79/22.31**, `target` **9.61/12.12**; **107.57 GiB free**. All ceilings
pass (`run62-disk.json`, raw whole-checkout du/df). No further artifact-heavy
work follows this audit. Handoff: `report-run62.md`.

### Lane I iso17: named-subtree stdout SVG files (2026-10-25)

Read the newest reports/messages, STATUS and the full brief including its
initial Update and **Update 27**. The isometric API sweep is closed. Chose
Run 61's six failing `internal-link-tooltip` / `node-edge-links` named-subtree
stdout cases across Dagre/ELK/TALA. No commits; other lanes and `29` preserved.

Go **d2cli/board_output.go:109–116,159–165** appends the selected board's
name before **main.go:892–904** tests for stdout; `-` becomes a literal
directory. Native recursive SVG output now follows that order, preserves the
empty extension in files and links, and skips transactional staging/manifest
publication for `-`, as **main.go:724–733** does. The implementation change
is confined to 15 added / 3 removed lines in CLI `main.rs`.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Target/link complete source contracts | **66/72 → 72/72** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Shared-clock/release CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Flag matrix | **304/304 → unchanged** | **0** |
| Isometric CLI / watch stages | **74 / 20 → unchanged** | **0** |
| Shared-clock cli-diff | **118/119 → unchanged** | **0** |

**235 native CLI + 4 existing harness tests pass**. The compressed executable
repeats all 235 tests; the compressed CLI repeats all six corrected calls with
complete inventories/bytes/logs. All SVGs and board manifests are validated.
No new tolerance, normalization, exclusion or golden claim. Initial missing
Pillow/websocket dependencies and an ordinary-oracle versus shared-clock PDF
profile mismatch are corrected setup failures, not passing evidence.

Evidence: `iso17-{links-before,links-final,e2e-before,e2e-final}.json`,
`iso17-deltas.json`, corrected export guards, `iso17-source.{json,patch}`,
`iso17-corpus-check.json` and test logs. `target-links.json` now points to this
qualification; the previous report/hash survives in `iso17-latest-retirement.json`.
Latest input/output hashes and explicit duplicate-retirement replacements verify.
Capture guards remain **1 GiB total / 128 MiB file / 20 GiB free**.
Other language, standalone renderer/API and full raster tiers are inherited.

**Next:** Update 27's **11 SVG + 2 color functions remain uncovered (13 total)**;
this run captures no new upstream function. Begin with `lib/svg/text_test.go`
attribute parsing/serialization and scoped IDs, then the two gradient functions.
GIF/timeline and imgbundler/imageasset stay with their owning lanes.

Cleanup and final whole-checkout allocated/apparent audit:
`iso17-{cleanup,compression,disk}.json`. No bulk moved or concurrent net savings
attributed. Exact duplicate captures are retired with checksum-bound replacements;
negative witnesses remain. Applesauce **0.5.30 / LZFSE** preserves hashes,
modes, ownership and original xattrs; temporary clone backups are removed.

Final allocated/apparent GiB: checkout **21.92/36.16**, `.reference` **10.81/22.33**, `target` **9.61/12.19**; **107.19 GiB free**. Every ceiling passes. Actual deletion: **373 files / 2.80 MiB allocated path footprint**; verified LZFSE saves **66.19 MiB**. Whole scans include hidden paths/.git and every apparent hard-linked path during concurrent lane activity. No further artifact-heavy work follows this audit.

### Run 61: selected-board links in ordinary SVG commands (2026-10-25)

Read the full brief through **Update 25**, newest run/I/L reports and STATUS;
also read the arriving L16/L17/I13 handoffs and **Update 26**. Chose L15's measured
`internal-link-tooltip.d2` failure, then clustered its root/named-target
variants across all three engines. No commits; other lanes and `29` preserved.

Static comparison with **d2cli/main.go:550–558,704–714,778–863** found that
Go clears selected descendants before building the full-root link map. Rust
now does the same for both node and connection links while retaining other
branches. Internal recursive hash traversal keeps those child subtrees.
Stdout board links preserve the destination's empty extension
(**board_output.go:109–116**), and multiboard stdout uses the exact
**main.go:902** error. No fixture-specific output or normalization was added.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New target/link source corpus | **30 → 66/72** | **0** |
| E2e SVG / errors | **933 / 11 → unchanged** | **0** |
| Isometric CLI / watch stages | **74 / 20 → unchanged** | **0** |
| Ordinary strict CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Flag matrix | **304/304 → unchanged** | **0** |
| cli-diff | **117/119 → 118/119 observed** | **0** |

The new corpus compares complete SVGs/logs/inventories and independently
validates board manifests: **24 SVG fixes + 12 exact-error fixes**. Its six
named-subtree stdout failures remain visible: Go writes extensionless files
under the literal `-/detail` directory; native still rejects them. The extra
cli-diff match is an uncontrolled PDF clock coincidence, not a source fix or
new stable qualification. The shared-clock/release CLI tier above is strict.

**233 native CLI + 37 harness tests pass**. Fixed a stale harness test that
mocked the retired v0.9.0 pin instead of `references.SOURCE_COMMIT`. Initial
harness setup failures and a rejected candidate hash regression are not
passing evidence; corrected final captures have zero regressions. Compressed
executables repeat the selected-board hash test and two complete CLI SVGs.
Scoped Ruff/diff checks pass; inherited Rust formatting is preserved.

Evidence: `run61-links-{baseline-final,final}.json` and complete captures,
`run61-deltas.json`, all fresh guard reports, test logs and
`run61-corpus-check.json` (**746 output bindings**, source inputs and current
runner hashes verified). New baseline: `target-links.json`; final executable:
`target/diagnostic/d2-rs-run61-qualified`. Existing aliases remain intact.
Captures retain **1 GiB total / 128 MiB file / 20 GiB free** streaming guards.
Language, full ordinary raster and standalone renderer/API tiers are inherited.

**Next:** Update 26's shared animated frame capture and GIF/timeline, the six
named-subtree stdout failures, the existing single-board animated SVG witness, then the
SVG-import ownership handoff. I13 also identifies shared custom-font
registration as a foundation dependency; require named failing boards before
expanding that work. Do not repeat closed static/paged/watch/flag work.

Cleanup and the fresh final whole-checkout disk audit are recorded in
`run61-cleanup.json`, `run61-compression.json` and `run61-disk.json`.
Actually removed **2,627 disposable files / 73.46 MiB allocated path
footprint**; verified Applesauce **0.5.30 LZFSE** saved **66.34 MiB** on four
idle single-link files. Checksums/modes/owners/original xattrs verify; clone
backups are removed. Final allocated/apparent GiB: checkout **21.54/35.57**,
`.reference` **10.67/22.16**, `target` **9.42/11.78**; **108.72 GiB free**.
Every ceiling passes. No bulk moved or concurrent volume/net savings claimed.
No further artifact-heavy work follows this audit.

### Lane I iso15: original rich-content targets through whole-image APIs (2026-10-25)

Read the newest reports/messages, STATUS and the full brief through Update
26, including its initial Update. Stayed within Lane I ownership. The
existing finite still-image corpora have no measured mismatch; custom-font
registration remains a shared foundation dependency. No production renderer
changes, foundation edits or commits in this run.

Added `--tier api --suite rich-boards`: the exact minimal targets constructed
by six original Markdown, light/dark code, class/SQL, opacity-budget and LaTeX
tests (`rich_labels.go:222–350`, `rich_labels_test.go`,
`surface_fidelity_test.go`) go through public SVG, PNG and RenderPage. The
original scene Build, returned document and assertions remain active. A
context marker prevents recursive observations without bypassing renderer
code. All **30/30 complete outputs match**, before and after; **0 regressions**.
These are constructed whole-board witnesses, not extra upstream goldens or
qualification of all original helper assertions, opacity controls or budgets.

Fresh ordinary e2e remains **933 SVG + 11 expected-error matches**, **0 fixed /
0 regressed**. Both API and e2e use unchanged hash-bound I13 runners; no new
Rust build or native-test claim. **Six original Go tests and 25 harness tests
pass**. Ruff on the API harness, gofmt and diff checks pass; the main
scoreboard retains its pre-existing SIM114 lint (also reproduced from HEAD).
Instrumented trace sources are restored and hooks detached. Other
isometric/foundation/language tiers
are inherited. Budgets remain streaming **1 GiB total / 128 MiB file /
20 GiB free**; no new normalization, exclusions or tolerances.

Inventory: **342 original img test functions, 28 with still-image boundary
observations (22 before), 314 without (320 before)**. Existing 102 original
public calls plus **162 constructed board calls**, 264 total, remain distinct.
GIF/timeline/capture callbacks and raster internals stay with Lane F.
Next Lane I candidates are the original embedded-asset/theme surface targets
(`surface_fidelity_test.go:222`) and Markdown-paper target
(`surface_content_test.go:12`); custom primary/mono registration still needs
the shared font-byte registry identified by I13.

Evidence: `iso15-rich-boards-{before,final}.json`, capture manifest and complete
final replay, `iso15-deltas.json`, `iso15-inventory.json`,
`iso15-corpus-check.json`, bounded e2e and harness logs. New alias:
`isometric-rich-boards-api.json`; prior aliases are preserved. All 30 new
input/result pairs, restored trace source hashes, runner hashes and duplicate
retirement replacements verify. No existing corpus artifacts were retired.

Cleanup actually removed **3 duplicate replay files / 1.17 MiB allocated path
footprint**; verified Applesauce **0.5.30 / LZFSE** saves **1.43 MiB** on
three idle single-link files. Hashes, modes, owners and original xattrs verify;
temporary clone backups removed (`iso15-{cleanup,compression}.json`). No
bulk moved or concurrent volume/net savings attributed.

Final allocated/apparent GiB: checkout **22.67/37.01**, `.reference` **11.81/23.32**, target **9.64/12.05**; **106.43 GiB free**. Every ceiling passes (`iso15-disk.json`, raw du/df). Hidden paths/.git and every apparent hard-linked path are included during concurrent lane activity. The repeat traversal completed without errors. No further artifact-heavy work follows this audit.

### Lane I iso14: remaining public still-image calls and link boards (2026-10-25)

Read the newest reports/messages, STATUS and the full brief through **Update
26**, including the initial Update. Stayed in Lane I. Added whole-image
observations of four original upstream test functions; **no renderer change
was needed** and no newly fixed boards are claimed.

The `links` API suite preserves the original assertions in
`page_test.go:172` and `animation_page_regression_test.go:105`, observing
board-target/file URL distinctions and outside Markdown captions through
complete SVG, PNG and RenderPage outputs. All **54/54** calls match.
The `still` suite observes the original public Render calls from
`TestNativeDeterministicCapture` and `TestSVGVisibilityLargeSources`, matching
**9/9**, including seven large original boards. Both repeat after capture
compression with **0 fixed / 0 regressed**. These are separate API witnesses,
not additional goldens. New latest aliases are `isometric-links-api.json`
and `isometric-still-api.json`; prior aliases remain unchanged.

Fresh ordinary e2e is **933 SVG / 11 expected errors**, **0 regressions**,
using the hash-qualified I13 runner. The native composition test now checks
all **54 link calls + 15 composition + 18 configured-font calls** and source
immutability; its **3 tests pass**, including from the compressed executable.
All **25 harness tests** and scoped format/lint/diff checks pass. Original
Go assertions pass, trace source is restored and hooks are detached.
Other isometric/foundation/language tiers are inherited; no production path
changed. Streaming guards remain **1 GiB total / 128 MiB per file / 20 GiB
free**; comparators, exclusions and timeouts are unchanged.

The first explicit img test-function inventory counts **342 original
functions**. **22** now have observed still-boundary calls (18 before), and
**320** have no capture in this API tier (324 before). This does **not** mean
every internal assertion in the 22 is qualified natively. Every original test
that directly calls Render/RenderPage is represented except
`TestNativeGIFWithoutTraffic`, owned by Lane F. Coverage is now **102 original
public calls + 132 whole-board witnesses**. Custom font registration remains
a shared foundation dependency; GIF/timeline remains Lane F's. Use the
explicit inventory to choose further original board/API witnesses, without
repeating passing still corpora or starting internal control sweeps.

Evidence: `iso14-{links,still}-{before,final}.json`, `iso14-deltas.json`,
`iso14-inventory.json`, `iso14-corpus-check.json`, bounded e2e and test logs,
and `report-iso14.md`. Permanent fixture: `tests/fixtures/link-boards-api.json`.
Cleanup removed **13 duplicate files / 123.65 MiB allocated path footprint**
(`iso14-cleanup.json`); retained inputs/expectations and runner hashes verify.
Applesauce **0.5.30 / verified LZFSE** saves **53.31 MiB** on four idle
single-link files, preserving bytes/modes/owners/original xattrs and removing
clone backups (`iso14-compression.json`, exact targets/rollback). No bulk moved
and no concurrent physical-volume/net savings attributed.

Final allocated/apparent GiB: checkout **21.59/35.71**, `.reference` **10.71/22.34**, target **9.50/12.02**; **108.72 GiB free**. Every ceiling passes (`iso14-disk.json`, raw du/df). Hidden paths/.git and every apparent hard-linked path are included during concurrent lane activity. No further artifact-heavy work follows this audit.

### Run 60: isometric watch previews and source reloads (2026-10-25)

Read the newest run/I/L handoffs, STATUS and the complete brief through
**Update 25**, then read the arriving I11/I12/L15 reports. The start scoreboard keeps
**933 SVG / 11 expected-error** matches. Static comparison found that GIF's
remaining implementation needs the shared animated frame-capture layer, so
selected the bounded watch command failure cluster instead. No run60 commit;
foreign lane edits and `29` are preserved.

`isometric_watch.rs` translates `d2cli/isometric_watch.go:17–99` and
`main.go:578–600`: prepare a static first-drawable-board preview before
publishing files, detach descendant slices for its hash, and rebase links
against the full source tree. File exports retain the selected subtree.
Two further source differences exposed by this corpus are fixed:
`isometric_image.go:245–296` resolves links to folder roots even when they
emit no image, and `main.go:1054–1057` gives ordinary static SVG files their final
newline while returning unchanged preview bytes. The public PNG convenience
entry point now delegates to I10's context-aware API; three original-board
1 ns deadline witnesses cover this String-returning boundary.

The new finite subprocess scoreboard drives real authenticated watch
HTTP/WebSocket sessions. It compares complete preview messages and file
bytes/inventories for source-mode reloads, root/leaf/sibling navigation and
folder-first previews, under SVG and PNG file exports. Every PNG passes the
existing raster gates, using isolated byte copies for watch's legitimate
single-file/previous-directory coexistence. Missing/corrupt output, missing
updates, timeouts and budget exhaustion fail. This does not execute every
original in-process watch assertion or qualify GIF watch.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New watch source/navigation stages | **0 → 20/20** | **0** |
| Isometric SVG/PNG CLI contracts | **50/50 → unchanged** | **0** |
| Isometric PDF/PPTX CLI contracts | **20 → 24/24** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Ordinary CLI release/shared-clock artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Flag matrix / cli-diff | **304/304 / 117/119 → unchanged** | **0** |

The four Architecture PDF/PPTX improvements come from Lane L's concurrent
shared compiler link/tooltip fix, requalified here with full document bytes
under the existing release/shared-clock profile. **233 CLI + 8 isometric +
47 harness tests pass**; the isometric tests include all twenty PNG fixture
boards. Compressed executables repeat two preview tests and four complete
SVG/PNG/PDF/PPTX artifacts. Scoped lint/format/diff checks pass. Language,
ordinary full-raster, standalone isometric SVG and other API tiers are
inherited; no new blanket parity claim is made.

Evidence: `run60-watch-negative.json`, `run60-watch-static-final.json`,
`run60-isometric-cli-static-qualified.json`, ordinary `*-static-final` guards,
qualified deltas, test logs and
`report-run60.md`. The corpora verify **272 output bindings**, 74 static/paged
inputs and 20 watch stages. Final CLI:
`target/diagnostic/d2-rs-run60-static-final`.
Every capture retains streaming **1 GiB total / 128 MiB file / 20 GiB free**
guards; ordinary combined runs reserve 64 MiB for compact evidence. Latest
aliases are detached before replacement, with previous bytes/hashes retained
in `run60-latest-retirement.json`.

**Next:** shared GIF frame capture, then GIF/timeline command output. The
qualified still `NativeScene` currently discards packets, animated-node ranges
and panel state needed by `geometry.go:687–728` / `native.go:24–90`; preserve
those source states before porting `gif.go`, `gif_loop.go` and `timeline.go`.
Do not repeat the closed watch/static/paged/ordinary flag corpora. The shared
SVG-import ownership handoff remains a separate Lane F request. L15 also hands
off the measured `--target=` internal-link rewrite witness in
`lang15-witness/README.md` (Go retains `root.layers.detail`, native emits
`detail.svg`). The final
static-newline review also records a pre-existing single-board animated SVG
source mismatch (`run60-animated-witness/`): both the run59 and final native
outputs are 5,346 bytes, versus Go's 10,077 bytes. It remains non-passing;
the final guard preserves the exact pre-run native bytes.

Cleanup ledger `run60-cleanup.json` removes **141 disposable files / 143.12 MiB
allocated path footprint**, preserving negative witnesses and reproduction
inputs. Verified Applesauce **0.5.30 / LZFSE** saves **89.98 MiB** on the final
idle CLI, CLI test executable and current release/shared-clock Go oracle;
byte hashes, modes, owners and original xattrs verify, and clone backups are
removed (`run60-static-compression.json`). The first compression pass is
superseded and excluded from these final compression savings.
No bulk moved; these savings do not attribute concurrent volume/net changes.
Final allocated/apparent GiB: checkout **21.38/35.27**, `.reference` **10.60/22.06**, target **9.33/11.58**; **107.19 GiB free**. Every ceiling passes (`run60-disk.json`, raw final du/df). Hidden paths/.git and all apparent hard-linked paths are included during concurrent lane activity. No further artifact-heavy work follows this audit.

### Run 59: isometric PDF/PPTX command paths (2026-10-25)

Continued Lane F after fully reading the brief through **Update 25**, the
latest run/I/L handoffs and STATUS; also read the arriving L14/I10 reports.
The start e2e sweep kept **933 SVG / 11 expected-error** matches. Selected
the next command gap: native isometric PDF/PPTX, previously rejected before
rendering. No run59 commit; foreign work and `29` are preserved.

`isometric_paged.rs` now feeds the qualified native RenderPage API into the
existing PDF/PPTX packagers, following `d2cli/isometric_paged.go` and
`paged.go:428–705`. It preserves folder-aware page/title traversal, selected
subtree link rebasing, external URLs/tooltips, format-specific annotation
rounding, page fills and presentation metadata. Operation pixel/PNG/link/string
budgets accumulate across pages; encoding and context checks precede atomic
file publication. Failed exports preserve previous bytes and modes, and stdout
stays empty. The foundation probe also retains declared MIME diagnostics, or
the existing shared net/http sniff result, at `lib/imageasset/probe.go:112–118,
355–377`, fixing malformed second-board icon errors. SOURCE.md files map both.

The new finite corpus follows the source contracts in
`d2cli/isometric_paged_test.go`; it does not claim to execute every original
in-process assertion. PDF full bytes use the explicit shared UTC epoch
**946782245** and FPDF catalog-sort control; PPTX uses the release-stamped Go
oracle. No comparator normalization, exclusions, tolerances or timeouts changed.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New paged file/stdout/selection/failure cases | **0 → 20/24** | **0** |
| Existing isometric SVG/PNG CLI contracts | **50/50 → unchanged** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Ordinary CLI release/shared-clock artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Flag matrix / cli-diff | **304/304 / 117/119 → unchanged** | **0** |

Four complete artifacts remain explicit mismatches: **Architecture PDF/PPTX,
file and stdout**. Static comparison identifies an earlier shared compiler
measurement divergence: Go `d2graph.go:1644–1653` adds 64 pixels for any paired
link/tooltip; Rust `compiler/graph.rs:3935–3940` incorrectly requires an HTTP(S)
host. The Detail node is **151 px in Go / 87 px in Rust**. Lane F leaves this
Lane L-owned file untouched. `run59-witness/` retains a minimal source and both
SVGs; the current negative PDF/PPTX captures remain. No page-image workaround
or annotation adjustment hides this mismatch.

**231 CLI + five foundation tests and 45 harness tests pass**. The initial
new no-MIME test expected the wrong source label; the corrected expectation
uses Go `resolve.go:266`'s `data URI`, and all five resolver tests pass. Scoped
rustfmt/Ruff/diff checks pass. Verified compressed CLI PNG/PDF/PPTX outputs
remain byte-identical to their qualified captures; both new native paged tests
and all five resolver tests rerun from compressed executables. Language,
full ordinary raster and standalone isometric tiers are inherited.

Evidence: `run59-paged-{baseline,candidate}.json`,
`run59-isometric-cli-final.json`, `run59-deltas.json`, ordinary guards/deltas,
test logs and `report-run59.md`. Fresh captures use a shared **1 GiB total
(960 MiB work + 64 MiB evidence) / 128 MiB file / 20 GiB free** streaming guard.
The final corpus verifies **74 inputs / 128 output bindings**, all negative
captures, CLI manifest and frozen runner hash. Latest static/paged CLI and
ordinary aliases are detached before replacement; old bytes/hashes remain in
`run59-latest-retirement.json`. Frozen CLI:
`target/diagnostic/d2-rs-run59-qualified`.

**Next:** Lane L's paired internal-link/tooltip measurement fix closes the four
remaining artifacts; rerun paged plus language/e2e guards. Lane F then takes
GIF/timeline and watch previews. I10 requests routing the owned raster PNG
convenience entry point through its new context-aware API; preserve I10's
named-board deadline witnesses. The shared SVG-import ownership handoff remains.

Cleanup removed **five disposable files / 113.66 MiB allocated path footprint**,
with executable hashes/rebuilds and verified smoke-output replacements in
`run59-cleanup.json`. All source/input/expected-output and unresolved failure
witnesses survive. Applesauce **0.5.30 / verified LZFSE** saved **68.36 MiB** on
three idle single-link executables, preserving bytes/modes/owners/original
xattrs and removing clone backups. Exact targets and rollback:
`run59-compression.json`. Shared active caches and foreign artifacts remain;
no bulk moved, and path savings are not attributed physical-volume savings.
Final allocated/apparent GiB: checkout **20.89/34.43**, `.reference`
**10.41/21.86**, target **9.02/11.03**; **111.77 GiB free**. Every ceiling
passes (`run59-disk.json`, raw du/df). Scans include hidden paths/.git and every
apparent hard-link path during concurrent lane activity; one transient I11
e2e temp directory disappeared during the walk. The original CLI capture also
requalifies through `--reuse` after cleanup/compression, with zero delta. No
further artifact-heavy work by run59.

### Run 58: close the theme-300 chaos2 raster failures (2026-10-25)

Read the latest run/I/L handoffs and the full brief through **Update 25**.
Selected its two remaining measured ordinary-board failures:
`stable/chaos2` PNG under `--theme=300`, with and without `--sketch`.
Both differed in seven pixels of the `88` connection label. Lane F's
isometric paged/GIF/watch wiring remains queued; other lanes and `29` survive.

Static source comparison and original Go glyph contours identify the first
mismatch in `d2raster/geometry.go:385–417` (`controlsProgress`). Go 1.27 ARM64
rounds the first product and fuses the second in its projection/cross product
(assembly at lines 400–401). Separate operations produce exactly **0.125**
at the quadratic's tolerance boundary; Go produces **0.12500000000000003**
and subdivides once more. The shared scene curve predicates in
`d2-rs-export/src/scanline.rs` now preserve this arithmetic, including the
adjacent audited `pointLineDistance:382`. Native code has no board/glyph
special case. The minimal quadratic regression preserves the exact Go point.
SOURCE.md maps the translation. The Go diagnostic hook was restored byte-exactly.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Full flag matrix | **302 → 304/304** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Full raster Dagre / ELK / TALA | **317 / 313 / 313 → unchanged** | **0** |
| CLI release/shared-clock artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Isometric CLI source contracts | **50/50 → unchanged** | **0** |

Every ordinary raster board completed with the existing timeout and exact
inventory/pixel gates; five upstream Dagre failures remain non-passing.
The complete corrected chaos2 PNG also equals Go's encoder bytes. No
normalization, exclusion, timeout or tolerance changed. **216 exporter tests +
38 raster-harness tests pass**. Scoped rustfmt and diff checks pass; inherited
whole-file formatting differences were left alone. Language/golden/component
API tiers are inherited, not newly requalified by this arithmetic change.

Evidence: `run58-{flags,e2e}-{before,final}.*`, `run58-*-delta.*`, three full
raster reports, CLI/isometric CLI guards, test logs and the compact
`run58-witness/` source/contour/assembly records. Frozen native CLI:
`target/diagnostic/d2-rs-run58-qualified`. Final captures have a shared streaming
**1 GiB total (960 MiB work + 64 MiB evidence reserve) / 128 MiB file /
20 GiB free** guard. Start-of-session e2e and flag-matrix runs used the existing
finite harnesses; final monitored replays qualified their complete inventories.

**Next:** CLI PDF/PPTX through the qualified RenderPage API, then GIF/timeline
and watch previews; the shared SVG-import ownership handoff also remains.
No new renderer component qualification queue was opened. No commits.
Cleanup retired **11 files / 133.88 MiB allocated path footprint**; verified
Applesauce LZFSE saved **102.80 MiB** on two idle single-link executables,
preserving bytes/modes/owners/original xattrs and removing clone backups.
Compressed CLI PNG and native regression smokes pass. Exact paths, retirement
records and rollback: `run58-{cleanup,compression}.json`. No bulk moved.
Final allocated/apparent GiB: checkout **21.03/34.42**, `.reference` **10.43/21.91**, target **9.18/10.89**; **111.77 GiB free**. Every ceiling passes (`run58-disk.json`, raw du/df). Hidden paths/.git and every apparent hard-link path are included, during concurrent lane activity. No further artifact-heavy work by run 58.

### Run 57: isometric SVG/PNG through the CLI (2026-10-25)

Continued the latest handoffs and complete brief through Update 24, within
Lane F's CLI scope at master **6a7b48378**. `--isometric`, `$D2_ISOMETRIC`
and source configuration now select the qualified native SVG/PNG renderer.
The command preserves compiled layout, applies source/flag precedence and
format-specific scale validation, wraps and namespaces SVG document options,
rebases static board links, publishes multiboard manifests transactionally,
and preserves previous files/modes on single-image atomic writes.

Source mapping: `d2cli/{main,isometric,isometric_image}.go`,
`d2svg/isometric.go`, `d2exporter/export.go:47–58` and
`d2target.go:46–58,168–203`; details in the CLI SOURCE.md. The earliest
hash divergence was the current board's `append(nil, slice...)` copies versus
untouched descendants' allocated empty slices. Root-only Config and retained
subtree hash bytes now preserve both distinctions. Two exact Go byte-stream
fixtures guard this boundary. No comparator normalization or exclusion changed.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New finite isometric CLI source contracts | **3 → 50/50** | **0** |
| cli-diff (help closes two rows) | **115 → 117/119** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| CLI release/shared-clock artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |

The new tier checks **30 complete SVG/PNG cases + 20 complete errors**;
PNG uses both required inventory/pixel gates, and complete encoder bytes also
match. It is a source subprocess corpus, not execution of every upstream
in-process test. **229 native CLI + four new harness + 38 raster-harness tests
pass**; Ruff and scoped diff checks pass. Standalone isometric SVG/PNG renderer
corpora remain inherited. Latest new alias: `isometric-cli.json` under the
master scoreboard directory. Evidence: `run57-*-{before,final,delta}.json`,
complete before/final captures, tests, `run57-corpus-check.json` and report-run57.md.
The frozen CLI is `target/diagnostic/d2-rs-run57-qualified`; trace hash hooks
were restored exactly. Capture limits: **1 GiB / 128 MiB per file / 20 GiB free**.

**Next:** finish CLI PDF/PPTX using Lane I8's RenderPage API, then GIF/timeline
and watch previews. Those command paths currently return explicit unsupported
errors; this run does not claim complete isometric CLI/API parity. The requested
shared SVG-import ownership handoff also remains queued. No commits by run 57;
other-lane changes and `29` are preserved.

Cleanup retired **1,020 files / 113.94 MiB allocated path footprint**;
verified Applesauce **0.5.30 LZFSE** saved **32.45 MiB** on the single-link
frozen CLI. Hash/mode/owner/original xattrs and post-compression SVG/PNG byte
smokes verify; its clone backup is removed. Ledgers: `run57-cleanup.json`,
`run57-compression.json`. Original negatives and qualified input/output closure
survive; active shared caches/foreign artifacts were left alone. No bulk moved.
Final allocated/apparent GiB: checkout **20.44/33.60**, `.reference` **10.28/21.67**, target **8.73/10.31**; **115.59 GiB free**. Every ceiling passes (`run57-disk.json`, raw du/df). Hidden paths/.git and every apparent hard-link path are counted, including concurrent lane activity. No further artifact-heavy work by run 57.

### Lane L21: default-denied and rooted local bundling (2026-10-25)

Read newest reports/messages, STATUS and the full brief through **Update 27**,
then arriving I18, **Update 28** and F63. Captured all **five local-policy imgbundler functions /
eight calls**, retaining original Go assertions. Added the absent explicit
native policy wrapper: default denial, unrestricted opt-in, retained Unix
root descriptors, relative-symlink containment, policy-scoped cache keys and
shared close. Each deprecated wrapper invocation owns its resolver/cache;
root replacement cannot reuse an earlier document's cached bytes. Mapping:
**imgbundler.go:84–118,151–180**, **localfile.go:53–256**, **open_unix.go** and
Go 1.27 **os/root.go rootMaxSymlinks** at master **6a7b48378**.
No main.rs/isometric/color/SVG changes or commits by this lane.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Local-policy calls | **8 unsupported → 8 exact output/error matches** | **0** |
| Existing resolver calls | **12 exact + 1 assertion → unchanged** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Shared-clock/release CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Shared-clock cli-diff | **118/119 → unchanged** | **0** |

Real captured files/symlinks are reconstructed for each invocation; the
oversized sparse-zero input's complete hash is checked. Root-replacement
calls run consecutively in one native process. **242 native + 19 harness
tests pass**, including all native tests from compressed executables.
Native tests additionally cover pinned directory identity after rename,
relative symlinks, leading-parent rejection, cache scopes, shared close,
cancellation, bounded reads and nonblocking FIFO rejection. Compressed
runners repeat the full new/existing imgbundler calls and three complete
stable/direction SVGs. All trace test bytes are restored. No normalization,
exclusion or tolerance was added; strict documents keep the established UTC
epoch **946782245**, catalog sort and release profile. Other language,
flags, full raster and isometric tiers are inherited.

Source/replay/retirement closure: **475 files** verified; evidence under
`lang21-{before,qualified,resolver,deltas,source,corpus-check}.json`, original
`lang21-policy-capture`, `lang21-work`, compact source patch, and
`report-lang21.md`. New alias: `imgbundler-local-policy-api.json`; previous
aliases survive. Capture limits: **1 GiB total / 128 MiB file / 20 GiB free**.
Initial cache-signature build failure was corrected. Scoped formatting/lint
passes; inherited unrelated image_resolver formatting is unchanged.

Inventory now **143 functions, 108 captured, 35 uncovered**: this run adds
five functions and arriving I18 adds two color functions. **Lane L 29 → 24**:
imgbundler **1**, localfile **8**, netpolicy **8**, imageasset **7**. Lane I
retains **11 SVG** functions. Next: TestBundleRemoteNetworkPolicy, then
localfile's eight functions. Unix rooted APIs are implemented; other rooted
platforms, arbitrary OS error spelling and close-error reporting are not
qualified. L20's automatic timer/logger gap remains.

Cleanup actually removes **70 files / at least 285.67 MiB measured allocated
path footprint**, plus one unmeasured initial test-executable retirement
explicitly recorded in `lang21-cleanup.json`. Successful duplicates and
superseded idle Cargo variants retire; input/expected-output closure and
negative baselines survive. Verified Applesauce **0.5.30 LZFSE** saves
**73.69 MiB** on 11 retained idle files; hashes, modes, owners and original
xattrs verify and backups are removed. No bulk moved or concurrent net
savings claimed.

Final allocated/apparent GiB: checkout **22.10/36.61**, `.reference` **10.87/22.38**, `target` **9.78/12.63**; **107.19 GiB free**. Every ceiling passes (`lang21-disk.json`, raw du/df); scans include hidden paths/.git and every apparent hard-linked path during concurrent activity. No further artifact-heavy work follows this audit.

### Lane L20: imgbundler resolver and failure contracts (2026-10-25)

Continued L19 and the full brief through **Update 26**, then read arriving
**Update 27**, F62 and I17. The SVG/color functions have moved to Lane I;
this run changes only Lane L's imgbundler API, adapter and harness. No
`main.rs`, isometric, policy/transport implementation edits or commits.

Seven original test functions now capture **13 calls**. The absent native
resolver/worker adapter was explicitly unsupported on all 13; the caller-owned
Rust API now gives **12 exact output/error matches + one scheduling assertion
match**. Mapping: master **imgbundler.go:59–80,296–389**. It composes the native
`RasterImageResolver` with canonical MIME/data-URI serialization, sticky
cumulative replacement limits, bounded workers, first-cause cancellation and
completed partial replacements. This adds API coverage, with no golden gain.

The scheduling witness's first-failing URL is nondeterministic in Go. Its
raw Go/Rust errors remain intact; that row separately checks upstream's
original predicates (malformed resolution failure, original SVG, request
count at most 16, original failure rather than context cancellation). It is
**not** an exact-error match. All other calls compare complete output/errors.
The cache test invokes the original Go helper/factory; native cache-enabled
and disabled calls run consecutively in one process with fresh per-document
resolvers. The hook records raw mock HTTP replies and local file bytes,
retaining every original Go assertion. It does not qualify rooted files,
network policy, transport ownership, arbitrary scheduling or timeout timing.
The CLI keeps its existing resolver/worker bridge. The new API accepts caller
cancellation; Go's automatic five-minute timer and logger ticker remain
unported at this API boundary.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New imgbundler resolver calls | **13 unsupported → 12 exact + 1 assertion** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Release/shared-clock CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Shared-clock cli-diff | **118/119 → unchanged** | **0** |

**240 native tests (235 CLI + 5 imgbundler) and 12 harness tests pass.**
Ruff, scoped rustfmt/gofmt and diff checks pass. The compressed runner repeats
all 13 calls; the compressed native test repeats partial replacement/failure
handling. Trace test bytes are restored. All **472 latest capture/replay
files**, runner and source hashes verify. Initial pre-batch-runner and
verification-script setup errors were corrected and are not passing evidence.
Existing explicit UTC epoch **946782245**, catalog sort and release stamp
control strict documents; no output normalization, tolerance or exclusion was
added. Language, flags, full raster and isometric tiers are inherited.

Inventory: **143 master-added functions, 101 captured, 42 uncovered** (was
94/49). Remaining imgbundler **6**, localfile **8**, netpolicy **8**, imageasset
**7** belong to Lane L (**29**). SVG **11** and color **2** belong to Lane I
(**13**) under Update 27. Next: five imgbundler local-policy functions and its
network-policy function, then localfile; do not repeat the completed language
queue or Lane I's SVG/color work.

Evidence: `lang20-{inventory,deltas,source,corpus-check}.json`, scoped source
patch, original `lang20-resolver-qualified-capture`, negative
`lang20-baseline-qualified.json`, final `lang20-qualified.json`, fresh guarded
reports under `lang20-work`, and `report-lang20.md`. New alias:
`imgbundler-resolver-api.json`; prior aliases survive. Capture guards:
**1 GiB total / 128 MiB file / 20 GiB free**. Superseded pilot/replay/runner
bindings are explicitly retired in `lang20-cleanup.json`; original qualified
inputs, expected results and negative baseline remain. No bulk moved.

Actually removed **106 disposable files / 3.78 MiB allocated path footprint**. Verified LZFSE saves **39.48 MiB** across 12 compression operations (including the explicitly retired trial runner), preserving hashes, modes, ownership and original xattrs; clone backups are removed. Final allocated/apparent GiB: checkout **21.88/36.17**, `.reference` **10.82/22.34**, `target` **9.62/12.25**; **106.81 GiB free**. Every ceiling passes (`lang20-disk.json`, raw du/df), including hidden paths/.git and all apparent hard-linked paths during concurrent lane activity. No further artifact-heavy work follows this audit.

### Lane L19: bounded imgbundler contracts (2026-10-25)

Continued the full brief through **Update 26**, with latest L17/F61/I14
reports. Six original imgbundler resource-limit test functions now have
**12 captured calls**, retaining every Go assertion. The previous scanner /
renderer replacement path matched **4**, mismatched **4**, and lacked **4**
internal API calls; the new shared Rust component matches **12/12**.
The pre-change adapter retains the exact old scanner body and calls the
existing native renderer; absent APIs remain explicitly unsupported.

`d2-rs-cli/src/imgbundler.rs` translates master `imgbundler.go:195–241,
251–291,368–436`: count eligible occurrences before deduplication, keep raw
href keys, apply sticky cumulative byte budgets, and admit output before
replacement. It reuses the shared Go-compatible URL parser. The CLI bridge
uses the existing resolver/workers and preserves completed replacements.
`main.rs` has only small delegation/import edits. No isometric implementation
or transport/localfile policy changes; no commits or comparison allowances.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New imgbundler calls | **4 → 12/12** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Release/shared-clock CLI artifacts / goldens | **67/70 / 51 → unchanged** | **0** |
| Shared-clock cli-diff | **118/119 → unchanged** | **0** |

**238 native + 5 harness tests pass.** Compressed runners repeat all 12
captured calls and five new Rust tests. Original trace tests are restored;
source/runner hashes and **445 latest input/output files** verify. Streaming
limits remain **1 GiB total / 128 MiB file / 20 GiB free**. One concurrent
isometric trace-hook restoration interrupted Go's release build; its failed
log survives, and all four release captures pass on retry. Uncontrolled
cli-diff PDF timestamps remain unstable; the established explicit clock
profile compares every byte. No new PDF match is claimed. Language, raster,
flags and isometric tiers are inherited.

Inventory: **143 master-added functions, 94 captured, 49 uncovered** (was
88/55). Remaining: imgbundler **13**, SVG **11**, localfile **8**, netpolicy
**8**, imageasset **7**, color **2**. Continue the remaining imgbundler
functions first. This tier uses explicit pre-resolved SVG fixture resources;
it does not qualify resolver policy, transport ownership, cache lifecycle or
worker scheduling. Those original functions remain uncovered.

Evidence: `lang19-{inventory,deltas,source,corpus-check}.json`, scoped source
patch, original `lang19-imgbundler-capture`, final guarded reports under
`lang19-final-work`, and `report-lang19.md`. New alias: `imgbundler-api.json`.
The cleanup ledger explicitly retires superseded candidate runner/replay
bindings; original inputs, expected outputs and negative witnesses survive.
Actual cleanup and final disk measurements are recorded below.

Cleanup actually removed **82 files / 70.14 MiB allocated path footprint**. Verified LZFSE saves **74.80 MiB** on **33 idle single-link files**; checksums, modes, owners and original xattrs verify, with clone backups removed. Final allocated/apparent GiB: checkout **22.72/37.01**, `.reference` **11.81/23.32**, target **9.64/12.05**; **106.43 GiB free**. Every ceiling passes (`lang19-disk.json`, raw du/df). Full scans include hidden paths/.git and every apparent hard-linked path during concurrent lane activity. No further artifact-heavy work follows this audit.

### Lane L17: Update 16 scope exhausted (2026-10-25)

Read the newest L16/run60/I12 reports and last messages, STATUS, and the
complete brief, now through **Update 26**. The current task explicitly limits
Lane L to **Update 16** ownership; Update 26 instead expands ownership to
asset/security implementations. Requested clarification of that conflict and
kept the explicit task restriction. No production changes, builds, Go hooks,
new API adapters or commits. Foreign work and `29` are preserved.

Fresh inventory from pristine `v0.9.0..6a7b48378` still has **143 master-added
functions, 88 captured, 55 uncovered**: imgbundler **19**, SVG **11**,
localfile **8**, netpolicy **8**, imageasset **7**, color **2**. The uncovered
set exactly equals L15/L16. No measured failing test remains in the current
Update 16 queue. No implementation progress is claimed. Avoid duplicating
those other crates' APIs or repeating the
passing language/API corpora. If Update 26 supersedes the task restriction,
its next batch is the **19 imgbundler functions**.

The required fresh e2e sweep keeps **933 SVG / 11 expected-error matches**;
delta versus L16 is **0 fixed / 0 regressed**, repeated after report
compression. Reused L15's hash-qualified CLI; all 17 L15 source bindings
verify unchanged. Streaming guard: **1 GiB total / 128 MiB file / 20 GiB free**.
Language **915 goldens / 445 editor results** and **276 API results/sessions**
are inherited from L16, not newly replayed. No comparison allowances changed.

Evidence: `lang17-inventory.json`, `lang17-e2e-work/{e2e,guard}.json`,
`lang17-e2e-delta.log`, and `report-lang17.md`. Latest aliases stay intact.
`lang17-cleanup.json` records removal of the verified temporary compression
backup and empty e2e scratch directory; preexisting artifact cleanup savings
are **0 bytes**. Applesauce **0.5.30 / verified LZFSE** saves **144 KiB** on
the new retained e2e JSON, with hash/mode/owner/original xattrs preserved and
backup removed (`lang17-compression.json`, exact rollback included). No bulk
moved or physical-volume savings attributed.

Final allocated/apparent GiB: checkout **21.68/35.62**, `.reference` **10.68/22.15**, target **9.55/11.83**; **108.72 GiB free**. Every ceiling passes (`lang17-disk.json`, raw du/df). No further artifact-heavy work follows this audit.

### Lane L16: scoped queue audit and regression replay (2026-10-25)

Read the complete brief through **Update 25**, newest lane reports/messages,
STATUS and the arriving run60 handoff. The Update 16 queue currently has
**no named failing tests in Lane L-owned implementations**. No production
edits, new test adapters, trace instrumentation or lang16 commits were needed.
L15's 17 source bindings and all 13 retained runner hashes still verify.

Recomputed the master-added function inventory directly from pristine
`v0.9.0..6a7b48378`: **143 functions, 88 captured, 55 uncovered**, unchanged
from L15. The remaining functions map to foundation/SVG implementations
outside this lane: imgbundler **19**, SVG attributes/IDs **11**, localfile
**8**, netpolicy **8**, imageasset **7**, gradient parsing/serialization **2**.
The compiler's gradient admission tests are already captured; the remaining
gradient functions exercise the foundation paint model and SVG writer.
Do not duplicate those APIs in a Lane L crate merely to reduce this count.
Captured functions do not imply complete internal assertion/performance parity.

| Replayed tier | Before → final | Regressions |
|---|---:|---:|
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| Fifteen master API capture groups | **276/276 results/sessions → unchanged** | **0** |

Reused the hash-qualified, source-unchanged binaries; no build or new oracle
capture was necessary. All original capture hashes/assertion-pass events,
934 language requests, 930 golden hashes, 16 latest language reports and
the retained 74-case CLI input/output closure verify. CLI/renderer outcomes
are inherited, not newly replayed. The e2e replay and native invocations retain
the existing streaming **1 GiB total / 128 MiB file / 20 GiB free** gates.
No normalizations, exclusions, tolerances or timeouts changed. Latest aliases
were left intact; fresh reports retain this run's evidence.

Evidence: `.reference/codex-runs/lang16-{inventory,summary,source-check,
corpus-check}.json`, all `lang16-*-final.json` reports and deltas,
`lang16-guards.py`, `lang16-verify.py`, and `report-lang16.md`.
Next Lane L work requires a new named compiler/language failure or an explicit
scope change. Run60's `--target=` href and single-board animation witnesses,
plus GIF/timeline, remain outside this lane.

Cleanup removed **611 disposable successful replay files / 53.20 MiB
allocated path footprint**. Original input/expectation closure, negative
baselines and current executables survive; exact replay hashes and replacement
instructions are in `lang16-cleanup.json`. Verified Applesauce **0.5.30 /
LZFSE** saved **0.48 MiB** on three retained reports/ledgers; hashes, modes,
owners and original xattrs verify, with clone backups removed. Exact targets
and rollback are in `lang16-compression.json`. No bulk moved or concurrent
volume/net savings attributed. Final whole-checkout audit: `lang16-disk.json`.
Final allocated/apparent GiB: checkout **21.38/35.29**, `.reference` **10.62/22.09**, target **9.33/11.56**; **107.57 GiB free**. Every ceiling passes (`lang16-disk.json`, raw du/df). Whole scans include hidden paths/.git and every apparent hard-linked path during concurrent lane activity. No further artifact-heavy work follows this audit.

### Lane L15: paired board-link sizing and final scoped master API batch (2026-10-25)

Read the latest handoffs, STATUS and complete brief through **Update 25**;
continued Update 16 at master **6a7b48378**, including arriving run59/I11.
No commits; other lanes and `29` are preserved.

Closed run59's four Architecture artifacts: **pdf/pptx-paged-file/stdout**.
Go `d2graph.go:1644–1653` reserves 64 px for every retained link paired with
Tooltip. Rust incorrectly required an HTTP(S) host. Measurement now reuses
the same post-validation link as the target (`compile.go:1726–1762`), keeping
unresolved links absent and authored-width exclusions intact. Native fitting
coverage includes inherited class decorations. A stale preexisting Markdown
assertion is corrected against master `d2ir/compile.go:798–835` and a pinned-Go
SVG witness: nested variables expose both `nested.x` and short `x` aliases.

Added public native `d2_rs_lib::urlenc::{encode,decode}` with master's 16 MiB
decode limit (`lib/urlenc/urlenc.go:15–62`), the shared Go encoder and existing
Go-derived native base64/Huffman reader. All 987 prior decoder fixtures still
match, with exact-boundary and other compression-level checks. The original
`TestDecodeRejectsOversizedScript` capture compares full encoded bytes, empty
rejection output and exact errors. The original
`TestMarkdownMatcherNoMatchStorageIsSparse` now measures real native allocations
with a test-only thread counter and an allocation control. Go uses zero and
Rust one allocation per no-match scan; both satisfy the original <=1 bound.
Full text, work/error results and sparse terminal storage also match; raw
allocation observations remain. Production substitution logic is unchanged.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Two-function scoped API batch | **1 → 3/3 results; 1 → 2/2 functions** | **0** |
| Paged isometric CLI artifacts/errors | **20 → 24/24** | **0** |
| All isometric CLI source contracts | **70 → 74/74** | **0** |
| E2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| Every earlier master language capture | **unchanged** | **0** |

**155 native + 57 harness tests pass**, including all native tests again after
compression. One preexisting library ignore is unchanged. Current graph-aware
runners and CLI were rebuilt; the other language captures reuse the qualified
unchanged runners. Scoped rustfmt/Ruff/gofmt/diff checks pass. Capture writers
use **1 GiB total / 128 MiB file / 20 GiB free** streaming guards. Original Go
assertions stay active; L15 test hooks are detached and their source restored.
The preexisting clock hook belongs to the concurrent CLI lane and was left
intact. PDF retains the explicit UTC epoch 946782245 / catalog-order profile;
PPTX retains the release-stamped oracle. No comparison allowances changed.

The uncovered master-function inventory shrinks **57 → 55**, now mapping to
foundation/SVG paths outside Lane L. This finite inventory does not establish
full language/internal API or performance parity. The new library URL API is
not yet wired into WASM's private decoder; cross-scope consolidation remains.
A separate ordinary CLI `--target=` href mismatch remains on the minimal
internal-link witness: Go preserves `root.layers.detail`, native emits
`detail.svg`; both now size its node to **117×66**. Both raw negative SVGs
survive for Lane F; no whole-SVG match is claimed for that witness. All four
named paged artifacts match complete bytes. The Markdown minimal SVG matches.

Evidence: `.reference/codex-runs/lang15-remaining-{before,qualified}.json`,
qualified delta, `lang15-isometric-cli-{final,delta}.json`, all language/e2e
reports and deltas, original capture, compact permanent URL fixture,
`lang15-witness/README.md`, `lang15-rebuild.txt`, `lang15-language.patch` and
`report-lang15.md`. After cleanup/compression, fifteen captured tiers (including the original compiler),
934 language requests, 930 golden hashes, 74 CLI inputs / 128 output hashes,
16 latest language aliases and 13 current runner bindings verify. Complete
compressed CLI SVG/PDF smoke bytes match the retained Go output.

Cleanup actually retired **19 files / 218.20 MiB allocated path footprint**,
including superseded L14/baseline executables, exact-reconstructible replay
requests and a disposable Go oracle. Negative baselines and input/expectation
closure survive; historical hashes/rebuilds have explicit replacement records.
Verified Applesauce **0.5.30 LZFSE** saved **174.30 MiB** on 19 retained files,
with checksums/modes/owners/original xattrs preserved and clone backups removed.
Shared caches and foreign artifacts remain; no bulk moved and no
concurrent physical-volume/net savings attributed. Exact ledgers and rollback:
`lang15-cleanup.json`, `lang15-compression.json`. Final disk audit:
`lang15-disk.json` (whole checkout, hidden paths/.git and every apparent link).
Final allocated/apparent GiB: checkout **21.94/35.28**, `.reference` **11.08/22.06**, target **9.54/11.58**; **106.81 GiB free**. Every ceiling passes (`lang15-disk.json`, raw du/df). No further artifact-heavy work follows this audit.

### Lane L14: shared grid and safe-link APIs; recovered L13 batch (2026-10-25)

Read the newest reports, full brief through **Update 25**, and STATUS. Continued
Update 16's language/compiler scope at pinned master **6a7b48378**. L13 had
stopped on model capacity before its guards/report; its production changes,
seven-function capture and negative baselines are preserved and verified here.
No commits; other lanes and `29` are preserved.

The next four-function batch captures every original assertion and **42 full
results** from `TestGridCapacityLimits`, `TestEnsureFieldCreateWithoutCompiler`,
`TestCompositeContainsNodeHandlesExistingCycle` and `TestDangerousLinkPolicy`.
Baseline: **20 matches / 22 unsupported**. Added public
`d2_rs_core::grid::grid_capacity` (`d2graph/grid_diagram.go:5–32`), sharing
positive-axis, dimension and overflow-safe cell admission with compiler
validation. Added `markdown::link_safety::safe_markdown_link`
(`lib/textmeasure/markdown_svg.go:317–326`), retaining trimmed original safe
metadata. Existing cycle termination and compiler-free field creation already
match; the cycle capture preserves node identities rather than flattening them.
All **42/42** results now match: **22 gains, zero regressions**. Native tests
also cover nonpositive-axis precedence and trimming/encoded metadata.

Recovered L13 adds `Compiler::reserve_variable_expansion_aliases` and nil-map
admission (`d2ir/expansion.go:163–265`), preserving shared cumulative budgets,
scalar-value identity charges and typed context causes. Four raw budget
primitives and three direct array/edge cancellation tests now have original
captures: **12 → 14/14 sessions**, zero regressions. All reservation errors
compare exactly. Cancellation uses the original partial-progress/existence
predicates and retains raw observations; equal poll schedules are not claimed.
The stale L13 graph-capture pointer was corrected to the retained L11 source.

**99 native tests and 54 harness tests pass**; scoped rustfmt, Ruff, gofmt and
diff checks pass after correcting initial module-order/lint findings. Original
trace sources are restored and hooks detached. The uncovered master-function
inventory falls **68 → 57** (seven recovered L13 functions plus four new
functions); the remaining list includes foundation functions outside Lane L.
`TestMarkdownMatcherNoMatchStorageIsSparse` still has an uncaptured allocation
assertion. General internal API/performance parity remains incomplete.

Evidence: `.reference/codex-runs/lang14-{small,internal}-{before,final,delta}.json`
(the internal baseline is `lang13-internal-qualified-before.json`), permanent
`master-small.json` and `master-ir-internal.json` fixtures, all native/harness
logs, `lang14-uncovered-tests.json`, and `report-lang14.md`. Capture budgets:
**1 GiB total / 128 MiB per file / 20 GiB free**, monitored while writers run.
All prior master captures are unchanged: compiler/parser/clone/Markdown
**70/4/65/13**, glob/variable/edge/import/depth **14/17/11/2/3**, class/graph/copy
**2/7/9**. Full language/editor remains **915 goldens / 445 source+errors**;
rebuilt ordinary e2e remains **933 SVG / 11 errors**. Every delta has zero
regressions. Fifteen latest language aliases bind eleven current frozen
runners. After cleanup/compression, original inputs, expectations, 934 language
requests, 930 golden hashes, fixture bytes and current/historical bindings
verify (`lang14-corpus-check.json`). All 99 compressed native tests and the
compressed CLI version smoke pass. Other-lane tiers are inherited.

Cleanup retired **984 files / 277.77 MiB measured allocated path footprint**;
verified Applesauce **0.5.30 LZFSE** saved **125.11 MiB** on 24 idle single-link
executables. Hashes/modes/owners/original xattrs verify and backups are removed.
The first cleanup hit Python's recursion limit after 142 duplicate replay
files were deleted before ledger persistence. Their paths were recovered;
exact requests reconstruct, but old log hashes/footprints were not persisted.
Those footprints are excluded from measured savings. Original Go inputs and
expectations and all negative witnesses survive. The retry increases recursion
capacity and checkpoints each deletion. Superseded executable/warm-path
bindings have retirement records. Exact paths, replacements and rollback:
`lang14-cleanup.json`, `lang14-warm-path-retirement.json`,
`lang14-compression.json`. No bulk moved; no concurrent physical-volume/net
savings attributed. Final allocated/apparent GiB: checkout **20.96/34.46**, `.reference` **10.38/21.83**, target **9.17/11.01**; **112.15 GiB free**. Every ceiling passes (`lang14-disk.json`, raw du/df). Scans include hidden paths/.git and every apparent hard-link path during concurrent lane activity. No further artifact-heavy work follows this audit.

### Lane L12: post-Compile IR copy reservations (2026-10-25)

Continued the full brief through Update 23 within Update 16's Lane L scope,
at master **6a7b48378**. Captured three original IR functions with every Go
assertion active: `TestVariableExpansionCopyUnitsIncludeSlices`,
`TestVariableExpansionCopyReservationsAreCumulative`, and
`TestImportedArraySpreadChargesInsertedMembers`. Baseline: **eight matching
results / one unsupported copy session**. Existing node costs and the exact
21-unit imported-spread difference already matched; those are new evidence,
not algorithm gains.

Added the missing native `Compiler::reserve_variable_expansion_copy` API,
following `d2ir/expansion.go:180–265`. It shares the compilation-wide
counter with substitutions and graph lowering, checks context before charging
each logical occurrence, and preserves typed causes and caller IR. A rejected
node leaves earlier charges consumed: the original successive-copy witness
uses **0 → 7 → 9** units, with the second call returning the limit error.
The existing graph copy path shares the same walk/reservation primitive.
Read-only usage/node-cost methods expose the existing accounting. SOURCE.md
maps the translation; no output normalization or exclusions changed.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original copy/accounting results / complete functions | **8 → 9/9; 2 → 3/3** | **0** |
| Compiler / parser / clone / Markdown calls | **70 / 4 / 65 / 13 → unchanged** | **0** |
| Glob / variable / edge / import / depth IR | **14 / 17 / 11 / 2 / 3 → unchanged** | **0** |
| Class-index / graph-materialization calls | **2 / 7 → unchanged** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| Rebuilt e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

**90 native + 47 harness tests pass**, including the four new native tests.
They cover all original results, typed cancellation/deadlines, partial budget
consumption, independent retry contexts, nested maps and repeated DAG
occurrences. Scoped rustfmt/Ruff/Go formatting/diff checks pass. Two previously
uncaptured post-Compile assertions are now observed completely. **68**
inventoried functions remain uncaptured, including three direct internal
compiler cancellation tests and four budget primitives; some remaining lib
functions belong outside this lane. Measure those before choosing the next
fix by failing-test count. General IR context/performance parity is not claimed.

Evidence: `.reference/codex-runs/lang12-copy-{before,final,delta}.json`, all
language/e2e guards and deltas, native/harness logs, `lang12-corpus-check.json`
and `report-lang12.md`. Thirteen latest language reports bind ten frozen
runners. Original captures, 934 language requests, 930 golden hashes, the new
fixture and current runner bindings verify after cleanup/compression. The
trace test is restored exactly; its hook is detached. Captures retained the
**1 GiB total / 128 MiB file / 20 GiB free** limits; both e2e guards pass.
Other-lane tiers are inherited. No commits; foreign work and `29` preserved.

Cleanup retired **458 files / 48.03 MiB allocated path footprint** of verified
duplicate replays and seven superseded language examples. Historical hashes/
rebuilds and warm-path replacements have retirement records. Shared caches and
prior CLI/Markdown bindings remain for concurrent lanes; no bulk moved.
Applesauce **0.5.30**, verified **LZFSE**, saved **106.31 MiB** on 24 idle
single-link retained files, preserving bytes/modes/owners/original xattrs; clone
backups removed. All 90 compressed native tests and CLI smoke pass. Exact
targets/rollback and deletion ledger: `lang12-compression.json` /
`lang12-cleanup.json`. These are path savings, not attributed volume savings.

Final allocated/apparent GiB: checkout **20.45/33.54**, `.reference` **10.41/21.63**, `target` **8.61/10.14**; **29.53 GiB free**. Every ceiling passes
(`lang12-disk.json`, raw du/df). Scans include hidden paths/.git and every
apparent hard-link path during concurrent lane activity. No further
artifact-heavy work by L12.

### Lane L11: graph materialization bounds and cancellation (2026-10-25)

Continued the newest handoffs and the full brief through Update 22, within
Update 16's language/compiler scope at master **6a7b48378**. The original
`TestGraphMaterializationBoundsUnusedAliasesAcrossBoards` cases for layers,
scenarios and steps, and `TestCompileCancellationDuringGraphMaterialization`,
exposed **four failing subtests**. Ordinary boards matched, but aliased boards
escaped the copy bound and graph lowering ignored cancellation.

`compiler/graph_materialization.rs` and the connected native graph paths follow
`d2compiler/compile.go:113–225` and `d2ir/expansion.go:180–265`: reserve every
logical occurrence before automatic board/folder copies, share the original
IR budget across boards and repeated calls, and propagate a latched context
through board, class and property views. Immutable IR views retain lexical
board identity. The first detached-copy attempt broke three existing link/order
tests; preserving that identity corrected the earliest divergence. Failed
logs remain. No partial graph is published on cancellation or budget failure.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original graph calls / complete subtests | **3 → 7/7; 0 → 4/4** | **0** |
| Compiler / parser / clone / Markdown calls | **70 / 4 / 65 / 13 → unchanged** | **0** |
| Glob / variable / edge / import / depth IR | **14 / 17 / 11 / 2 / 3 → unchanged** | **0** |
| Original class-index sessions | **2 → 2/2** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

**86 native + 44 harness tests pass**, with scoped rustfmt/Ruff/diff checks.
The original Go assertions stay active. Captures retain the actual shared
composite identities, full graph/error results and typed context counts;
the permanent fixture preserves all seven calls exactly. No allowance,
normalization, exclusion or timeout changed. Other-lane tiers are inherited.

Evidence: `.reference/codex-runs/lang11-graph-{before,qualified-final,delta}.json`,
all language/e2e guards and deltas, native/harness logs,
`lang11-corpus-check.json` and `report-lang11.md`. Twelve latest language
reports bind nine runners. After cleanup, original captured inputs/expectations,
934 language requests, 930 bound golden hashes, the fixture, warm tests and
latest/historical report bindings verify. The trace test is restored exactly;
its temporary hook is detached. Capture limits stayed **1 GiB total /
128 MiB file / 20 GiB free**; both e2e streaming guards pass.

**Open:** 69 inventoried functions remain uncaptured (some outside this lane),
plus two post-Compile IR assertions. The three previously uncovered compiler
leaf subtests are now captured. Next measure the remaining internal IR budgets,
copy units and cancellation contracts, selecting by actual failing-test count.
General graph polling and performance complexity remain unqualified.

Cleanup retired **1,208 files / 426.24 MiB allocated path footprint**:
verified duplicate replays, superseded language examples and idle oracle-profile
tests/codegen. The historical test hashes and rebuilds have explicit replacement
records in `lang11-cleanup.json`; the first oracle lib run had replaced its
old warm path. Current diagnostic-profile tests, original negatives and input/
expectation closure remain. Shared caches and the bound L10 CLI/Markdown runner
remain for concurrent lanes. No bulk moved; no physical-volume savings are
attributed from these path figures. Applesauce **0.5.30**, verified **LZFSE**,
saved **103.12 MiB** on 32 newly compressed idle single-link retained files;
the previously compressed Markdown runner also verifies with zero size change.
Hashes, modes, owners and original xattrs verify, backups are removed, and all
86 compressed native tests plus CLI smoke pass. Exact targets/rollback:
`lang11-compression.json`.
No commits by this lane; foreign work and `29` preserved.

Final allocated/apparent GiB: checkout **19.96/32.94**, `.reference` **10.14/21.43**, target **8.34/9.86**; **29.49 GiB free**. Every ceiling passes (`lang11-disk.json`, raw du/df). Hidden paths and every apparent hard-link path are counted; scans include concurrent lane activity. No further artifact-heavy work by L11.

### Lane L10: reusable, cancelable compiler class indexes (2026-10-25)

Continued the latest lane handoffs and the full brief through Update 21,
within Update 16's Lane L scope at master **6a7b48378**. Captured the original
`TestClassLookupBuildsOneReusableIndex` and
`TestClassLookupIndexConstructionIsCancelable`, with all assertions active.
Both complete sessions initially failed: ordinary lookup results were correct
but no reusable index existed; canceled construction returned a class map.

`compiler/class_index.rs` now follows `d2compiler/compile.go:570–651`:
one source-ordered index per root, first match including nil maps, quoted
reserved-name filtering, Unicode simple-fold keys, context checks before
each member and before publication, and no partial index on failure.
`ClassLookup` borrows immutable IR and retains typed context causes. Graph
class resolution shares the implementation; property-overlay views reuse
their containing session's cache. The core exports its existing fold key.
Both SOURCE files map the change.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Complete original class-index sessions (8,197 full lookup results) | **0 → 2/2** | **0** |
| Compiler / parser / clone / Markdown calls | **70 / 4 / 65 / 13 → unchanged** | **0** |
| Glob / variable / edge / import / depth IR calls | **14 / 17 / 11 / 2 / 3 → unchanged** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

**82 native tests + 41 harness checks pass**, as do scoped rustfmt/Ruff/diff
checks. The permanent fixture losslessly dictionary-encodes every original
result and restores it before comparison. Checks also cover typed deadlines,
discarded partial indexes, empty roots, cache reuse, Unicode and nil shadows.
No comparator allowance, normalization, exclusion or timeout changed.
The replay driver needed a higher Python recursion capacity for deep parser
ASTs; failed attempts remain recorded. Native test-only assertion/quoted-node
fixture errors were corrected before the final passing run.

Evidence: `.reference/codex-runs/lang10-class-qualified-{before,final,delta}.json`,
all language/e2e guards and deltas, `lang10-corpus-check.json`, tests and
`report-lang10.md`. Eleven latest language reports bind eight frozen runners.
Original captures, 934 language requests, 930 bound golden hashes and the new
fixture verify after cleanup/compression; all 82 tests also pass from the
compressed warm executables. The trace test is restored exactly and the hook
detached. Capture limits: **1 GiB total / 128 MiB file / 20 GiB free**.

**Open:** 71 inventoried functions (some outside this lane), three compiler
leaf subtests and two post-Compile IR assertions remain uncaptured. Next:
graph-materialization cancellation/copy budgets, then internal IR contracts,
selected by newly measured failing-test count. General graph context parity
and performance complexity are not established by these two class tests.

Cleanup retired **1,182 files / 437.12 MiB allocated path footprint**:
verified duplicate captures/replays, six superseded L9 examples and completed
own Cargo codegen. Negative outputs have retained exact-byte replacements;
inputs/expectations, warm tests and other-lane CLI bindings survive. Ledger:
`lang10-cleanup.json`. Applesauce **0.5.30**, verified **LZFSE**, saved
**126.70 MiB** on 24 idle single-link retained files, preserving bytes, modes,
ownership and original xattrs; backups were removed. Exact targets and rollback:
`lang10-compression.json`. Shared caches remain during concurrent lanes;
no bulk moved. These are path measurements, not attributed volume savings.
No commits by L10; foreign work and `29` preserved.

Final allocated/apparent GiB: checkout **19.52/32.38**, `.reference` **9.98/21.26**, target **8.03/9.48**; **30.10 GiB free**. Every ceiling passes (`lang10-disk.json`, raw du/df). Full scans include hidden paths and all apparent hard-link paths; target/free refreshed afterward. Sequential measurements include concurrent lane activity. No further artifact-heavy work by L10.

### Lane L9: Markdown substitution contexts (2026-10-25)

Continued the newest handoffs and full brief through Update 21 within Update
16's Lane L scope at master **6a7b48378**. Captured **13 calls from seven
original `lib/textmeasure/substitutions_test.go` functions**, retaining all Go
assertions, including the 10,000-pattern timeout subprocess. Initial results
were **10 matches / three unsupported context calls**. Selected that sole
measured cluster: `TestReplaceSubstitutionsMarkdownBoundedCancellation`,
`TestReplaceSubstitutionsMarkdownIgnoresUnreferencedVariablesInLinearTime`,
and `TestReplaceSubstitutionsMarkdownMatchesManyPatternsInLinearTime`.

The text crate now exposes signed-limit, context-aware bounded replacement.
It checks during matcher construction, Markdown enter/exit visits, byte and
candidate scans, selection and assembly (`lib/textmeasure/substitutions.go:
24–332`). Typed context causes discard partial text/work. Negative limits
precede context checks; the background API shares the implementation.
The compiler forwards its context and latches cancellation/deadline causes
(`d2ir/compile.go:605–622`). SOURCE files map both changes.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original Markdown calls, complete result/work/typed errors | **10 → 13 / 13** | **0** |
| Public compiler / parser / clone calls | **70 / 4 / 65 → unchanged** | **0** |
| Glob / variable / edge / import / depth IR calls | **14 / 17 / 11 / 2 / 3 → unchanged** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

**124 native tests (46 text + 78 language) and 37 harness checks pass**;
scoped rustfmt, Ruff and diff checks pass. All 13 requests/results are a
permanent fixture. Additional tests cover negative-limit precedence, typed
deadlines, discarded work across text nodes and reuse after resource failure.
No normalization, allowance, exclusion or timeout changed. Native matcher
allocation counts and general complexity remain unqualified. **73 inventoried
functions**, three compiler leaf subtests and two post-Compile IR assertions
remain uncaptured. Next capture graph-materialization/class-index and internal
IR cancellation/budget contracts, then select by actual failing-test count.

Evidence: `.reference/codex-runs/lang9-markdown-{before,final,delta}.json`,
guard reports/deltas, native/harness logs, `lang9-validation.json` and
`report-lang9.md`. Ten latest language reports bind seven frozen runners.
All prior capture hashes, 934 language requests, 930 golden hashes and the
new fixture verify after cleanup (`lang9-corpus-check.json`). The original
trace test is restored exactly and the hook detached. The initial zero-call
Go capture failed; corrected name matching produced the fresh v2 capture.
Capture limits remained **1 GiB total / 128 MiB file / 20 GiB free**.

Cleanup retired **423 files**, including five idle superseded L8 examples,
with **23.28 MiB measured allocated path footprint**. An initial cleanup
verifier hit Python's deep-AST recursion limit after 144 verified duplicate
deletions; their recovered path/replacement ledger excludes those unmeasured
savings. The verifier's recursion capacity was raised and the complete
closure passed. `lang9-cleanup.json` preserves that distinction. Original
negative outputs, current inputs/expectations and other-lane CLI bindings
survive. Shared caches/Cargo variants remain during concurrent work; no bulk
moved. No compression pass: `.reference` remained below the brief's 10 GiB
trigger. No commits by L9; other lanes and `29` preserved.

Final allocated/apparent GiB: checkout **19.10/31.74**, `.reference` **9.68/20.88**, target **8.09/9.30**; **30.36 GiB free**. All ceilings pass (`lang9-disk.json`). Whole-checkout scans include hidden paths; apparent counts every hard-linked path. Sequential scans include concurrent lane writes; target/free refreshed after full scans.

### Lane L8: cancelable cached-import AST clones (2026-10-25)

Continued the newest reports and full brief through Update 21 within Update
16's Lane L scope at master **6a7b48378**. Captured the two remaining original
cached-AST cancellation tests with every Go assertion active. All **65 calls**
were inventoried: **29 matches, 36 mismatches** initially. Selected that sole
measured cluster, `TestCachedImportASTCloneStopsOnCancellation` and
`TestCachedImportASTArrayCloneCancellationNeverReturnsPartialValues`.

The core's `clone_ast_with_context` now follows `d2ir/import_cache.go:13–382`:
check context before node/box/slice work, also after copying a numeric rational,
and discard the entire working tree on failure. Existing background cloning
uses the same implementation. `compiler/import_compile.rs` connects it after
the AST-copy reservation, preserving the typed cause and halting compilation
without returning a partial tree (`d2ir/import.go:226–265`). Source mappings are
in the core/language SOURCE files. Imported parsing and other internal context
APIs remain unqualified.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original cached-AST clone calls, complete AST/typed errors | **29 → 65 / 65** | **0** |
| Public compiler / parser calls | **70 / 4 → unchanged** | **0** |
| Glob / variable / edge / import / depth IR calls | **14 / 17 / 11 / 2 / 3 → unchanged** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

**145 native tests (78 language + 67 core) and 33 harness checks pass**, along
with scoped rustfmt, Ruff and diff checks. The permanent 65-call fixture
preserves full original requests/results. Additional native coverage checks
deadline causes, discarded partial clones and source ownership across repeated
cancellation and edits. No comparison allowance, normalization, exclusion or
timeout changed. The compressed clone tests and CLI smoke also pass.

Evidence: `.reference/codex-runs/lang8-clone-{before,final,delta}.json`, all
guard final reports/deltas, `lang8-validation.json`, test/build logs and
`report-lang8.md`. Nine latest language reports bind six frozen L8 runners.
The complete input/expectation closure, 930 bound golden hashes, 11 warm test
hashes and 36 original negative outputs verify after cleanup
(`lang8-corpus-check.json`). Trace sources are restored exactly; hook detached.
**Open:** 80 uncaptured functions (including foundation tests), three compiler
leaf subtests and two post-Compile IR assertions. Capture the remaining internal
IR cancellation/budget and compiler materialization contracts, then select by
actual failing-test count. These finite calls do not establish full master
language/API parity.

An initial free-space stop paused parity development. Scoped cleanup plus a
fresh full passing audit (`lang8-disk-resume.json`) preceded the e2e guard,
capture and builds. Capture limits remained **1 GiB total / 128 MiB per file /
20 GiB free**; the final e2e streaming guard passes. The first Go capture's
unused import was corrected before the fresh 65-call capture; the failed
zero-call attempt remains non-passing.

Cleanup retired **5,430 files / 853.20 MiB allocated path footprint**: idle Go
actions, completed own codegen, superseded positive language executables and
strictly verified successful replay duplicates. Inputs, expected outputs,
negative baselines and other-lane bindings survive; retirement overlays are
explicit. Applesauce **0.5.30**, verified **LZFSE**, saved **89.51 MiB** on
17 current retained executables, preserving hashes/modes/owners/original xattrs
and removing all clone backups. An earlier eight-file pass saved 29.87 MiB;
those old executables were then retired after qualification, so that amount
is not counted as current retained-path savings. These are path measurements,
not attributed physical-volume or concurrent-checkout savings. No bulk moved.
Final allocated/apparent GiB: checkout **18.20/30.66**,
`.reference` **9.57/20.73**, `target` **7.28/8.37**; **29.34 GiB free**.
Every ceiling passes (`lang8-disk.json`). Sequential scans include concurrent
lane writes; target/free were refreshed afterward. No further artifact-heavy
work by L8. No commits; other lanes and `29` preserved.

### Lane L7: master active import-chain bound (2026-10-25)

Continued the newest lane reports and full brief through Update 21, within
Update 16's Lane L scope at master **6a7b48378**. Captured the four remaining
public import/depth functions with all original Go assertions active,
including the 10,000-file subprocess. Five calls first measured **three
matches and two depth mismatches**. Selected that sole failing cluster:
`TestImportDepthLimit#0` and `TestImportDepthBoundary#1`.

Ported `d2ir/import.go:17–19,21–55` into `compiler/imports.rs`: a maximum of
**128 imported files per active chain**, excluding the root source. Path
validation/resolution precedes the bound; depth errors precede cyclic-chain
errors and never push a rejected path. The 129-file input previously succeeded;
the deep input previously failed later with a different work-limit diagnostic.
Both now produce the exact upstream error at `level-00127.d2:1:1`.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original import-depth calls, full IR/imports/errors | **1 → 3 / 3** | **0** |
| Original import context/disabled-FS calls | **2 → 2 / 2** | **0** |
| Public compiler / parser calls | **70 / 4 → unchanged** | **0** |
| Glob / variable / edge IR calls | **14 / 17 / 11 → unchanged** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

**76 native tests + 29 harness checks pass**, with scoped rustfmt, Ruff and
diff checks. Five complete original requests/results are permanent native
fixtures. Additional tests check root exclusion, active-chain counting,
depth-versus-cycle precedence and unchanged stacks after rejection. No
normalization, exclusion, comparison allowance or timeout changed. The import
capture describes cancellation on filesystem open without advancing the
original context; typed cancellation and complete outputs already match.
Consumption during imported parsing/cloning remains unqualified.

Evidence: `.reference/codex-runs/lang7-{depth,import}-before.json`, qualified
reports and deltas, master/parser/language/e2e final reports and deltas,
`lang7-validation.json`, native/harness logs and `report-lang7.md`. Eight
latest language reports now bind five frozen L7 runners. Trace source files
are restored exactly and the temporary hook is detached. Six IR captures,
the parser capture, 70 compiler calls, 934 language cases and 930 bound
goldens verify after cleanup. L6 executable retirements and prior latest
aliases have explicit overlays; original depth negatives remain.

**Open:** 82 uncaptured functions (including foundation-library tests outside
this lane), three compiler leaf subtests and two post-Compile IR assertions.
Next capture internal variable/edge cancellation and graph materialization
assertions, then choose measured failures by count. These finite calls do
not establish full master language/API or context parity.

Capture limits remain **1 GiB total / 128 MiB per file / 20 GiB free**.
An external volume-space drop made the replay guard fail closed; those
attempts remain non-passing. Own queued builds were stopped, and a fresh full
passing disk audit preceded all resumed checks. Final e2e streaming guard
passes. Cleanup, compression and final disk: `lang7-cleanup.json`,
`lang7-compression.json`, `lang7-final-compression.json`, `lang7-disk.json`.
No commits; other lanes and `29` preserved.

Cleanup retired **841 files, 510.66 MiB allocated path footprint**. Verified
LZFSE on nine idle single-link artifacts saved another **42.82 MiB allocated**;
hashes/modes/owners/original xattrs match and all clone backups are removed.
These path measurements do not imply physical-volume savings or attribute
concurrent cleanup to this lane. Original negatives and retained closures verify.
Final allocated/apparent GiB: checkout **18.64/30.90**, `.reference`
**10.44/21.60**, `target` **6.76/7.70**; **30.98 GiB free**.
Every ceiling passes (`lang7-disk.json`); no further artifact-heavy work followed.

### Lane L6: master parser nesting bounds and typed contexts (2026-10-25)

Continued the latest lane handoffs and full brief through Update 19, within
Update 16's language scope at master **6a7b48378**. Existing measured calls
all passed. Captured the three new original parser functions with every Go
assertion intact: four calls first scored **one match, two nesting mismatches
and one unsupported cancellation call**. Fixed the largest cluster first,
`TestParseNestingDepthLimit/map#0` and `/array#0`, then
`TestParseContextCanceled#0`. The supported-depth call remains exact.

Ported the shared **128-level map/array nesting limit**, excluding the file
map. Overflow halts rune consumption, preserves partial ASTs/ranges and
suppresses later termination diagnostics. Added `parse_with_context` and
`parse_stream_with_context`, checking before the first rune attempt and every
256 attempts thereafter. Typed context causes latch and take precedence over
syntax diagnostics; context-free result types remain intact. Static source:
`d2parser/parse.go:22–24,48–101,220–269,443–460,1601–1617`; mapping in language
`SOURCE.md`.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New parser calls, full partial AST/typed errors/diagnostics | **1 → 4 / 4** | **0** |
| Public compiler calls | **70 → 70 / 70** | **0** |
| Variable / glob / edge IR calls | **17 / 14 / 11 → unchanged** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

**73 native tests** (62 library, seven existing integrations, four new parser
contracts) and **28 harness checks** pass; scoped rustfmt, Ruff and diff checks
pass. New native tests cover original ranges/boundaries, mixed nesting,
10,000-level input, typed deadlines, partial results, latched cancellation
and syntax accumulators through both input boundaries. No exclusion,
normalization, timeout or comparison allowance changed. Only Python's
recursion capacity is raised so the existing strict comparator can traverse
the full supported-depth AST. Integrity checks reject missing leaf calls even
with refreshed hashes. The original trace test and its pre-existing golden
observer are restored exactly; the temporary hook is detached.

Evidence: `.reference/codex-runs/lang6-parser-{baseline,final,delta}.json`,
master/variable/glob/edge/language/e2e final reports and deltas, test logs,
`lang6-validation.json` and `lang6-corpus-check.json`. Four IR captures, four
new parser calls, 70 compiler requests, 934 language requests and 930 bound
goldens verify after cleanup. Latest language reports are atomically replaced
and bind frozen L6 runners; prior aliases have an explicit overlay. Original
native negatives and the compact baseline adapter remain. Capture limits:
**1 GiB total / 128 MiB file / 20 GiB free**; final e2e streaming guard passes.
No commits; foreign work and `29` preserved.

**Open:** 86 inventoried uncaptured functions (including foundation-library
tests outside this lane), three compiler leaf subtests and two post-Compile
IR assertions. Next capture internal variable/edge context assertions and
import-parser contexts, then select actual failures by count. This finite
tier does not establish arbitrary-context, import propagation, graph
materialization or full master language/API parity.

Cleanup retired **1,039 files**, **465.49 MiB allocated path footprint**;
short scoped du reductions total **458.04 MiB** (`lang6-cleanup.json`). Removed
completed codegen, two obsolete editor dependency variants, superseded L5
language runners and verified successful replay duplicates. Inputs, expected
results, negatives and current runners verify. L4/L5 CLIs remain for active
iso4 bindings; shared Go actions remain during concurrent work. Nothing moved.
A temporary target overage was cleaned, and a fresh passing whole-checkout
audit preceded resumed builds. Final disk: `report-lang6.md`, `lang6-disk.json`.

When the final scan put `.reference` above the brief's 10 GiB compression
trigger, Applesauce **0.5.30** verified LZFSE compression on nine idle,
single-link L6 runners/capture files, saving **40.08 MiB allocated**. SHA-256,
modes, owners and non-compression xattrs remain identical; all temporary
clone backups were removed immediately. `lang6-compression.json` records exact
targets, before/after metadata and rollback. The five runner hashes and full
parser capture verify again; the compressed CLI's version smoke passes.

Final allocated/apparent GiB: checkout **21.03/32.36**, `.reference`
**9.68/20.83**, `target` **9.93/9.93**; **36.58 GiB free**. Every ceiling
passes (`lang6-disk.json` and raw du/df). Target/free were refreshed after the
full post-compression scans; sequential snapshots include concurrent lane
writes. No further artifact-heavy work followed this audit.

### Lane L5: cooperative cancellation during IR construction (2026-10-25)

Continued the latest lane reports and full brief through Update 19, within
Update 16's language scope at master **6a7b48378**. Targeted the sole measured
failure, `TestCompileCancellationDuringIRConstruction#0`. The new edge suite
first measured **11/11** original Compile calls from eight functions; its four
internal-only functions remain explicitly uncovered, with all Go assertions
retained. The variable capture now records the original context's threshold
and initial check count without advancing it. All 17 prior requests/results
are preserved, with only additional context/edge-option metadata.

Added `CompileContext` / `compile_ir_with_context`, sharing the existing stop
gates in construction, expansion and lazy replay. Cancellation latches the
original typed cause and returns its exact bare diagnostic before accumulated
source errors. Existing entry points retain their options/defaults. Static
mapping: `d2ir/compile.go:108–195`, `d2ir/expansion.go:56–80`; details in language
`SOURCE.md`. Parsing and graph materialization remain separate context stages.
No comparator, normalization, exclusion or timeout changed.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original variable IR calls, full IR/imports/errors | **16 → 17 / 17** | **0** |
| Newly captured edge IR calls | **11 → 11 / 11** | **0** |
| Public compiler / existing glob IR calls | **70 / 14 → unchanged** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

**69 native tests** (62 library, four existing integrations, three new context
contracts) and **24 harness checks** pass; scoped Ruff/rustfmt/diff checks pass.
The new fixture retains the original cancellation request/result; native tests
also check partial construction, pre-canceled precedence over invalid limits
and import I/O, typed deadline causes and source-error precedence. Four IR
captures, all 930 bound goldens and the unchanged 70-call compiler capture
verify. Trace test files are restored exactly and the hook is detached.

The combined inventory leaves **89 uncaptured functions**, three internal
compiler leaf subtests and two IR post-Compile assertions. Next capture the
internal variable/edge context assertions and parser/import contexts, then
choose actual failures by count. This finite call tier does not establish
complete master language/API or cancellation parity.

Evidence: `.reference/codex-runs/lang5-{variable,edge}-{before,final,delta}.json`,
glob/master/language/e2e final reports and deltas, validation and test logs;
handoff `report-lang5.md`. Latest language reports point to frozen L5 runners.
No commits; other lanes and `29` preserved. Cleanup/disk: `lang5-cleanup.json`
and `lang5-disk.json` (final audit below).

Cleanup retired **892 files**, **544.32 MiB allocated path footprint**; short scoped du reductions total **543.60 MiB** (`lang5-cleanup.json`). Superseded runners, completed codegen and successful replay duplicates were removed. Original negatives, inputs/expected outputs and current executables verify after cleanup. The L4 CLI remains for active iso4/run50 bindings. Nothing moved; compression deferred under Update 3 while `.reference` remains below 10 GiB allocated.

Final allocated/apparent GiB: checkout **21.11/32.34**, `.reference` **9.79/20.87**, `target` **9.83/9.80**; **26.17 GiB free**. Every ceiling passes (`lang5-disk.json`, raw du/df). These sequential snapshots include concurrent lane writes; no physical-volume savings are inferred. No further artifact-heavy work followed the audit.

### Lane L4: master import expansion accounting (2026-10-25)

Continued the full brief through Update 19 and the latest lane reports within
Update 16's scope. Captured the original `d2ir/expansion_test.go` assertions:
17 Compile calls from 11 of 16 functions. The largest measured cluster was
two import-budget failures: `TestImportedVariableExpansionUsesCompilationBudget#0`
and `TestRepeatedImportPeeksUseCompilationBudget#1`. Imports now reserve AST
copies, every peek, cached selected-field/map copies, new IR templates and
array-spread members against the same compilation budget before materializing
them. Imported substitution errors restore the saved diagnostics before
stopping; the independent limit error remains available for the public result.

Static source mapping: `d2ir/import.go:86–163,191–265`,
`d2ir/expansion.go:98–125,318–380`, and
`d2ir/compile.go:202–214,2339–2342`; language `SOURCE.md` records it.
Public import-cache loaders retain independent AST copies. No exclusion,
normalization, timeout or comparator allowance changed.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New original variable IR calls, full IR/imports/errors | **14 → 16 / 17** | **0** |
| Master public compiler calls | **70 → 70 / 70** | **0** |
| Existing glob IR calls | **14 → 14 / 14** | **0** |
| Language goldens / editor source+errors | **915 / 445 → unchanged** | **0** |
| e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

The one remaining variable call, `TestCompileCancellationDuringIRConstruction#0`,
is explicitly unsupported. Five internal-only functions and two post-Compile
assertions remain unqualified by this call tier. The original spread-budget
assertion also passes separately in a native unit: **21** extra work units.
All original Go assertions passed; the trace test is restored byte-for-byte
and its hook detached. Five full original import results are permanent native
fixtures. **62 library tests, four integration tests and 23 harness tests**
pass; focused Ruff, rustfmt and diff checks pass. Final CLI/runners were
rebuilt and frozen before replay. Both IR captures, the original 70 compiler
requests and all bound master language golden hashes verify after cleanup.

The combined inventory leaves **97 uncaptured original functions**, three
compiler leaf subtests and two IR post-Compile assertions. Next address the
named cancellation failure, first capturing a deterministic cancellation
policy rather than just the current `has_context` flag. Then instrument the
12 original edge-expansion functions and choose measured failures by count.
These finite calls do not establish full master language/API compatibility.

Evidence: `.reference/codex-runs/lang4-{ir-before,ir-final,ir-delta,
master-final,master-delta,glob-ir-final,glob-ir-delta,language-final,
e2e-final,validation,uncovered}.json`, language/e2e delta text and native logs.
Latest language baselines updated atomically; prior bindings have an explicit
overlay. Handoff: `report-lang4.md`. No commits by this run; other lanes and
the stray `29` are preserved.

Cleanup retired **936 idle/redundant files**, with **578.25 MiB** gross
measured scoped du reduction (`lang4-cleanup.json`): completed thin-LTO
intermediates, superseded executables and successful replay duplicates.
Original requests/expected results, original negative logs and the unsupported
cancellation witness survive. No bulk moved. Compression remains deferred
under brief Update 3 while `.reference` is below 10 GiB allocated. Target
briefly exceeded its ceiling during test linking; cleanup and a fresh passing
whole-checkout audit preceded resumed builds.

Final allocated/apparent GiB: checkout **20.92/32.15**, `.reference` **9.70/20.75**, `target` **9.80/9.77**; **28.15 GiB free**. Every ceiling passes (`lang4-disk.json`, raw du/df). Sequential snapshots include concurrent lane writes; no further artifact-heavy work followed this audit.

### Lane L3: master glob work and materialization bounds (2026-10-25)

Continued the newest reports and full brief through Update 18, within Update
16's Lane L scope. Chose the sole measured public-call failure,
`TestCompilePropagatesGlobExpansionLimit#0`. Ported the compilation-wide
65,536-unit glob budget and independent 10,000-created-field ceiling.
Keys charge before context deduplication; traversal charges candidates before
filtering; generated fields reserve prospective depth for every active glob.
Substitution placeholders and array values reserve before materialization.
Exhaustion halts traversal with the exact source diagnostic. Full triple-glob
traversal now skips unnamed placeholders, following master's fixed nil guard.

Static mapping: `d2ir/glob_expansion.go:9–135`, `pattern.go:9–203`,
`compile.go:120–136,1043–1049,1161–1184,2284–2304`, and
`d2ir.go:1216–1222,1257–1309,1399–1445`. Core/language SOURCE files record it.
No fixture-specific behavior, normalization, exclusion or timeout changed.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Master public compiler calls, complete graph/config/errors | **69 → 70 / 70** | **0** |
| New original glob IR calls, full IR/imports/errors | **14 / 14 first qualification** | **0** between initial/final |
| Master language goldens | **915 → 915** | **0** |
| Editor source/errors | **445 → 445** | **0** |
| e2e SVG / expected errors | **933 / 11 → 933 / 11** | **0** |

The new env-gated hook retains all original Go assertions in
`d2ir/glob_expansion_test.go`; ten functions yield 14 calls, including the
2,000-field ordinary-glob control, exact array/placeholder boundaries,
negative options and default recursive feedback. The primitive budget test
has a native unit but remains explicitly uncovered by the call scoreboard.
Thirteen compact complete results are permanent fixtures; the large control
remains in `lang3-ir-capture`. Both runners share the existing full IR JSON
projection. Go test bytes are restored exactly and the hook is detached.

**169 library tests + 3 integration tests** and **22 harness tests** pass;
scoped Ruff, rustfmt and diff checks pass. Final runners and CLI were rebuilt
and frozen. The language guard uses master's case manifest (934 requests;
its request bytes equal the older manifest, whose pin metadata was v0.9.0).
All bound golden hashes and all original 70 compiler-capture hashes verify.
The combined inventory still has **108 uncovered test functions and three
internal compiler leaf subtests**. Next: instrument the 16 functions in
`d2ir/expansion_test.go`, then choose its failures by count; edge-budget tests
(12 functions), cancellation and import/graph-copy budgets remain open.
This closes the measured public-call gap, not full master language parity.

Evidence: `.reference/codex-runs/lang3-{master-qualified,master-delta,
ir-final,ir-delta,language-final,e2e-qualified,validation,uncovered}.json`,
`lang3-{language,e2e}-delta.txt`, native/harness logs and
`report-lang3.md`. Latest language reports are in
`.reference/scoreboard-master-6a7b4837/`. No commits by this run; other lanes'
work and `29` were preserved.

Cleanup retired **699 idle superseded files**, with **599.80 MiB** gross
measured scoped du reduction across five deletion ledgers. Executable
retirement overlays identify replacement qualification records; all original
inputs, expected outputs and negative baselines remain. Nothing moved.
Fresh allocated/apparent GiB: checkout **20.80/32.23**, `.reference`
**9.50/20.59**, `target` **9.76/9.73**; **36.35 GiB free**. Every ceiling
passes (`lang3-disk.json`, raw du/df). Target briefly crossed its ceiling
during builds; cleanup and a fresh full audit preceded further builds.
Concurrent lane changes are not attributed to own savings. Compression is
deferred per brief Update 3 while `.reference` stays below 10 GiB allocated.

### Lane L2: master compiler grid, link, config and edge bounds (2026-10-25)

Continued the full brief through Update 16 and the latest lane reports. Work
stayed within language/text and its own tools; foundation/isometric WIP and
the stray `29` were preserved. No commits by this run.

Chose the remaining groups by failing-call count: grid capacity **6**, links
**6**, isometric config **5**, edge budgets **4**, gradient stops **2**, and
disabled imports **1**. Grid axes now reject values above 10,000; capacity
rejects more than 1,000,000 cells at the later dimension key. Ordinary links
share the bounded HTML/percent/control normalization policy, and source edge
records promote an absent label from any safe link, including `mailto:`.
Source config validates and extracts isometric booleans; rendering remains
Lane I's scope. Edge globs reserve distinct segment/endpoint pairs before
self/container/board filtering and charge every lazy examination separately.
Explicit edges are uncharged. Gradient positions must be finite numbers or
percentages; a missing filesystem reports Go's exact import-node diagnostic.

Source mapping: `d2compiler/compile.go:164,217–247,1020–1076,1222–1224,
1838–1842`, `d2graph/grid_diagram.go:6–29`, `d2ir/edge_expansion.go:9–132`,
`d2ir/d2ir.go:1652,1797`, `d2ir/import.go:239–240`,
`lib/color/gradient.go:105–143`, and
`lib/textmeasure/markdown_svg.go:328–418`. Both crate SOURCE files record it.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Master public compiler calls, complete JSON/errors | **45 → 69 / 70** | **0** |
| Language goldens | **915 → 915** | **0** |
| Editor source/errors | **445 → 445** | **0** |
| e2e SVG / expected errors | **933 / 11 → 933 / 11** | **0** |

Intermediate master deltas are **45 → 51 → 57 → 62 → 66 → 69**, each with
zero regressions. The original Go capture is unchanged and hash-verified.
Final runners/CLI were rebuilt and frozen before scoring. All **102 library
tests**, **2 integration tests** covering **69 original compiler contracts**,
and **17 Python harness tests** pass. The text unit contract also retains all
17 upstream `TestDangerousLinkPolicy` inputs. Focused Ruff, rustfmt and diff
checks pass. No comparator allowance, exclusion or timeout changed.

The grid/gradient fixes exposed obsolete v0.9.0 unit-fixture expectations.
Env-gated master hooks independently reran every input: **807 color values**
(68 results changed), **88 color diagnostics** (9 changed), and **1,362 edge
diagnostics** (4 changed). New `*-master.json` fixtures retain the results;
historical fixtures and initial failing test log remain. Nothing was silently
accepted or removed from the tests.

The one remaining measured nonmatch is
`TestCompilePropagatesGlobExpansionLimit#0`, explicitly **unsupported**.
Port `d2ir/glob_expansion.go` and its traversal/materialization reservations
next, starting from its retained `**.a / **.b / **.c / x` input and 64-unit
error. **118 test functions and three internal leaf subtests remain uncovered**,
including cancellation and import/graph-copy accounting. This is finite
compiler-call evidence, not full master language/API parity.

Evidence: `.reference/codex-runs/lang2-{master-before,master-final,master-delta,
language-final,language-delta,e2e-before,e2e-final,e2e-delta,validation}.json`,
per-group deltas, native test logs and the fixture refresh deltas. Handoff:
`.reference/codex-runs/report-lang2.md`. Cleanup and fresh disk measurements
are recorded in `lang2-cleanup.json` and `lang2-disk.json`.

Cleanup retired **890 own superseded files**, with **344.84 MiB** gross
scoped du reduction. Original negatives, all inputs/expected results and
qualified snapshots remain; nothing moved. Shared action caches were left
alone during concurrent work. Final allocated/apparent GiB: checkout
**20.46/31.70**, `.reference` **9.33/20.42**, `target` **9.69/9.66**;
**33.91 GiB free**. Every ceiling passes. Measurements are sequential
snapshots; concurrent growth and volume-free-space shifts are not attributed
to this cleanup. Compression is deferred under brief Update 3 while
`.reference` remains below 10 GiB allocated.

### Lane L1: master compiler expansion bounds and cycles (2026-10-25)

Read the full Codex brief through Update 16 and the latest run-45 report.
Work is restricted to the language lane; renderer/foundation WIP is untouched.
`scripts/language-master-scoreboard.py` inventories **143 newly added test
functions** across the specified language packages and `lib/*`. It captures
**70 calls from 25 public compiler tests** through an env-gated wrapper in
`../d2-trace`, keeping each original Go assertion. Native comparison uses
complete graph/config JSON and exact ordered diagnostics, without normalization.
**118 test functions and three additional internal leaf subtests remain
explicitly uncovered**; a captured Compile call does not qualify a later
uncaptured internal assertion. This is a compiler-call tier, not all master
language/API parity.

The largest initial cluster was substitution work accounting (21 calls,
including its controls). Added `CompileLimits` / `compile_ir_with_limits`
without breaking existing filesystem-option initializers. Default compilation
uses upstream's 65,536-unit substitution limit. Reservations precede scalar
expansion, array/map spread copies, inherited-board/class copies and repeated
IR visits. A final alias walk rechecks mutable scalar/composite DAGs before
cleanup. Composite cycles and unresolved forward map spreads are rejected
before mutation. Markdown uses a complete-pattern Aho-Corasick matcher,
charges candidate matches and replacement bytes, and reserves flattened
variable patterns cumulatively. Source: master `d2ir/expansion.go:14–380`,
`d2ir/compile.go:216–353,416–848,897–916`, and
`lib/textmeasure/substitutions.go:23–332`; mappings are in both crate SOURCEs.

| Tier | Before → after | Regressions |
|---|---:|---:|
| Master public compiler calls, complete JSON/errors | **13 → 45 / 70** | **0** |
| Native crashes in those calls | **7 → 0** | — |
| Language goldens | **915 → 915** | **0** |
| Editor source/errors | **445 → 445** | **0** |
| e2e SVG / expected errors | **933 / 11 → 933 / 11** | **0** |

All **35 retained variable/cycle contracts** match, including success and
exact-boundary controls. Native tests: **101 library tests**, plus one
integration test checking those 35 original Go contracts; **17 Python harness
checks** pass. No test exclusion or comparator allowance was introduced.
The hook installer now preserves existing instrumentation; an install/uninstall
round trip verifies every affected compiler test file unchanged. The two
pre-existing compiler golden hooks were restored after the first installer
overwrote them. Lane L wrappers are detached at handoff; their reproducible
source remains in `tools/language-master-scoreboard/`.

Evidence: `.reference/codex-runs/lang1-{master-baseline,master-final,
master-delta,language-final,language-delta,e2e-before,e2e-final,e2e-delta}.json`,
original Go capture `lang1-master-capture/`, native before/after logs and
`crates/d2-rs-language/tests/fixtures/master-variable-expansion.json`.
Next measured gaps: **6 grid-capacity errors**, **6 link contracts**,
**5 isometric-config calls**, **4 edge-budget calls** (including the explicit
edge control), **1 glob-budget call**, **2 gradient-validation errors**, and **1 nil-filesystem import
contract**. These 25 nonmatches remain explicit (19 mismatches, 6 unsupported).
The broader uncovered queue includes cancellation, import/AST-copy limits,
graph-materialization copy accounting and internal IR budget APIs; this
checkpoint does not establish all substitution/security-bound behavior.

Cleanup ledger: **4,340 files retired**, with **627.94 MiB** of directly
measured scoped du reductions. The larger **1,048.99 MiB per-path allocation
inventory** is separate and is not claimed as recovered physical space.
Current negatives, inputs and final results remain; superseded native attempts,
idle Lane L codegen/library variants and idle Go actions were retired. Nothing
was moved out of the checkout. Final allocated/apparent GiB: checkout
**20.14/31.39**, `.reference` **9.16/20.25**, `target` **9.56/9.53**;
**40.44 GiB free**. Every ceiling passes (`lang1-disk.json`, raw du/df).
Concurrent lane growth is not attributed to this cleanup. Compression is
deferred under brief Update 3 while `.reference` remains below 10 GiB allocated.
No commits. Handoff: `report-lang1.md`.

## Isometric renderer and native scene foundations (2026-10-24; in progress)

Master's d2isometric (about 16k Go lines) builds on d2scene, d2scenebuild,
d2raster and internal/fontface, which d2-rs had only emulated through its
legacy SVG raster path. They are being ported natively, each against its own
Go-oracle tier over all 1008 e2e golden boards:

| Tier | Script | Status |
|---|---|---|
| d2scene model round trip | `scene-scoreboard.py --tier model` | **897/897** documents |
| d2isometric BuildScene | `isometric-scoreboard.py --tier scene` | **1008/1008** |
| isometric SVG visibility | `examples/isometric-visibility` | **870/870** |
| isometric SVG writer (Go mesh, surfaces stubbed) | `isometric-scoreboard.py --tier writer` | **870/870** (138 Go resource errors) |
| isometric SVG writer with native surfaces (Go mesh/document inputs) | `isometric-scoreboard.py --tier surfaces` | **870/870** complete SVGs; 138 Go resource errors remain non-passing |
| d2scenebuild documents | `scene-scoreboard.py --tier scenebuild` | **1008/1008** (897 documents, 111 build errors) |
| d2raster pixels | `scene-scoreboard.py --tier raster` | **896/896** images + **1/1** render-limit error; 111 boards have no scene input |
| fontface shaping/outlines | `fontface-scoreboard.py` | **5068/5068** corpus records |
| plain-label surfaces on actual Go mesh-builder calls | `isometric-scoreboard.py --tier text` | **762/762 nonempty boards; 9988/9988 calls** (108 explicit empty inventories, 138 Go errors) |
| rich-label painter calls from actual Go meshes | `isometric-scoreboard.py --tier rich` | **192/192 nonempty boards; 438/438 calls** (678 empty inventories, 138 Go errors; run 50 closes the LaTeX mismatches) |
| native mesh tessellation calls | `isometric-scoreboard.py --tier primitives` | **861/861 nonempty boards; 22585/22585 calls** (9 empty inventories, 138 Go errors; complete mesh assembly remains gated) |
| isometric PNG pixels / whole native PNG | `isometric-scoreboard.py --tier png` | **20/20 / 20/20** source-qualified boards (run56) |
| isometric SVG end to end | `isometric-scoreboard.py --tier svg` | **870/870** complete native SVGs; **138/138** source errors match (iso7/iso8) |

Crates: `d2-rs-target` (d2target JSON mirror, themes), `d2-rs-scene`,
`d2-rs-fontface`, `d2-rs-scenebuild`, `d2-rs-raster`, `d2-rs-isometric`.
Oracles: `tools/scene-probe`, `tools/isometric-probe` (including the
test-only mesh hook), built in `../d2-trace`. ARM64 contraction sites with
their enclosing compiled functions are tabulated from the pinned compiler's
assembly in `.reference/isometric-port/fma-sites*.txt`: fusion follows SSA
values (across variables and inlined helpers), not source expressions.

### Run 56: native isometric PNG goldens (2026-10-25)

Re-read the complete brief through Update 22 and newest lane handoffs. Lane F
moves from foundation sub-APIs to the 20 failing isometric PNG boards. Ported
`d2isometricimg/raster.go` static rendering and `opacity_groups.go` composition:
2x supersampling, bounded strips, depth/opacity shadow maps, hierarchy ground
receivers, texture interpolation, stable transparent ordering, object fades,
source-over/multiply and Go-compatible default PNG encoding. SOURCE.md maps
Go methods and audited ARM64 arithmetic. Ordinary raster meshes use their own
solid-cap mode through a minimal additive shared-builder entry point.

| Tier | Before → after | Regressions |
|---|---:|---:|
| All upstream PNG goldens on Go raster meshes | **0 → 20/20** | **0** |
| Complete native diagram → PNG | **0 → 20/20** | **0** |
| Complete native isometric SVG | **870 → 870/870** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All 20 PNGs reproduce Go's complete encoded bytes as well as exact decoded
pixels. They cover all-shapes, class/SQL, sequence/nested diagrams, opacity,
dark themes and all ten complex-layer boards. The PNG scoreboard first
reproduces every upstream golden with Go, then uses `collect_png_outputs` and
`compare_png_outputs` with additional exact pixel equality. Missing, corrupt,
empty, extra, timeout and budget failures cannot pass. Go-mesh and whole-native
paths are reported separately. Permanent fixtures retain all 11 original
compiled inputs and 20 source-qualified hashes; the complete public PNG path
is exercised by one inventory-checked native regression test.

Evidence: `.reference/codex-runs/run56-png-{before,complete}.json`, PNG/SVG/e2e
deltas, `run56-png-qualified-capture/manifest.json`, native and harness logs.
The SVG guard also observes **138/138** source errors now matching: the extra
87 are concurrent Lane I7's icon work, not a PNG gain. The e2e guard uses the
unchanged frozen CLI because this change does not yet link isometric into the
CLI. Other language/export/foundation tiers are inherited. No allowance,
fixture-specific output or diagnostic trace path was added. One intermediate
Go setup refused a differing concurrent mesh hook; the PNG producer now owns
an independent test hook and fresh qualified capture. Initial transient
foreign icon compilation failure was retried after that lane corrected it.

**23 native tests + four PNG-harness tests pass**, along with all **38**
existing `test_render_*.py` checks. Scoped rustfmt/Ruff/diff checks pass. Current
capture inputs, expectations, texture paths and frozen runner hashes verify
**504 bindings** after cleanup/compression (`run56-corpus-check.json`).
Capture budgets are **1 GiB total / 128 MiB per file / 20 GiB free**, with
streaming enforcement. The qualified standalone PNG hook is detached; foreign
trace hooks and lane edits are preserved. No commits by this lane.

Cleanup retired **1,552 files / 664.05 MiB allocated path footprint**, with
replacements/rebuilds in `run56-cleanup.json`; original PNG inputs/expectations,
current meshes/textures and final outputs remain. Applesauce **0.5.30**,
verified **LZFSE**, saved **132.52 MiB** across 371 idle single-link retained
files, preserving bytes, modes, ownership and original xattrs. Backups were
removed and three compressed runners pass exact-output smoke checks. Targets
and rollback are in `run56-final-compression.json`. No bulk moved; gross path
savings do not attribute shared-volume or concurrent-checkout net changes.

Static PNG is qualified for this finite corpus. Caller-context cancellation,
Raster.Frame/animation, general asset APIs, GIF/timeline and CLI `--isometric`
wiring remain open. Next in Update 22's queue: CLI flag/help/output wiring,
then GIF/timeline where a golden or original CLI test provides a failing case.

### Lane I iso13: configured-face errors and source composition (2026-10-25)

Read the newest I/F/L reports, STATUS and the full brief through **Update 25**.
Stayed within Lane I at master **6a7b48378**; no commits. Other lanes' changes
and `29` are preserved. The session-start e2e guard remains **933 SVG / 11
expected errors**.

Fixed **18 public-image error mismatches** on **unicode/with-style** and
**txtar/legend-mono**, each on Dagre/ELK/TALA through SVG, PNG and RenderPage.
The face lookup now shares the source family-role resolver before selecting
a style: an unavailable configured primary/mono face names its actual family
and style (`isometric text font <family>/bold is not loaded`), instead of
misreporting `DEFAULT` or `mono` as an unregistered explicit family. Mapping:
**surface_document.go:30–49; text.go:103–121**. No font discovery/substitution
or foundation, raster/GIF or CLI implementation changes by I13.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New configured-face public API witnesses | **0 → 18/18** | **0** |
| New original source-composition public calls | **15/15 → unchanged** | **0** |
| Existing Page / SVG / PNG / paint / panel API | **48 / 11 / 18 / 34 / 27 → unchanged** | **0** |
| Whole-board SVG / exact source errors | **870 / 138 → unchanged** | **0** |
| Explicit-resource SVG / native PNG | **9 / 20 → unchanged** | **0** |
| Rebuilt isometric CLI / e2e | **50 / 933+11 → unchanged** | **0** |

The composition hook runs all five original legend, sequence and decorated-root
helper inputs through the three public still-image boundaries, preserving
all original assertions. Complete SVG/PNG bytes, annotations and exact errors
compare. These **33 new board witnesses add no goldens** and do not qualify
animated frames. The two compact native fixtures also check input immutability.
Coverage is now **93 original public calls + 78 board witnesses**.

**39 native + 25 harness tests pass**; scoped format/lint/diff checks pass.
The original trace sources are restored and hooks detached. API/render captures
and the rebuilt e2e replay retain streaming **1 GiB total / 128 MiB per file /
20 GiB free** guards. No comparison allowances, exclusions or tolerances changed.
Evidence: `iso13-fonts-{before,final}.json`, `iso13-composition-{before,final}.json`,
`iso13-deltas.json`, existing guards, native/harness logs and corpus verification.
New aliases: `isometric-fonts-api.json`, `isometric-composition-api.json`.

**Next foundation request:** registered custom primary/mono font bytes need
one shared scene-builder registration API before the isometric plain/rich
paths can support them consistently (`surface_fidelity_test.go:64–106`).
This is a source-identified limitation, not a newly qualified custom-font tier.
GIF/timeline/CLI and the shared SVG-import ownership handoff remain Lane F's.
Other language/foundation/component tiers are inherited.

Cleanup ledger `iso13-cleanup.json` removes **1,021 duplicate files / 489.53 MiB
allocated path footprint**, preserving expectations, negative outputs and the
verified corpus closure. Applesauce **0.5.30 / verified LZFSE** saves
**210.09 MiB** on 24 idle single-link files; byte hashes, modes, owners and
original xattrs verify and clone backups are removed (`iso13-compression.json`).
All 39 native tests and both new API corpora repeat after compression.
No bulk moved; savings do not attribute concurrent physical-volume/net changes.

Final allocated/apparent GiB: checkout **21.68/35.62**, `.reference` **10.68/22.15**, target **9.55/11.83**; **108.72 GiB free**. Every ceiling passes (`iso13-disk.json`, raw du/df). Full scans include hidden paths/.git and every apparent hard-linked path during concurrent lane activity. No further artifact-heavy work follows this audit.

### Lane I iso12: legend-panel asset handoff (2026-10-25)

Continued the full brief through **Update 25**, newest lane reports and STATUS
at master **6a7b48378**, within Lane I ownership. Selected 27 public-image
failures on the original **txtar/legend-mono Dagre/ELK/TALA** boards with an
authored legend icon. No commits; foreign work and `29` are preserved.

Source panels dropped the caller's asset options and created no default
embedded-image resolver. The shared panel builder now receives the explicit
resolver, clamps caller SVG limits/budgets through the existing icon-painter
constructor, and rejects external icons before default loading. Both legend
and root-frame panels receive the handoff. Source mapping:
`surface_document.go:46–111`, `surface_fonts.go:19–54`,
`surface_icons.go:60–133`, `source_panels.go:128–145,210–223`; recorded in
isometric SOURCE.md. No geometry/foundation/raster/GIF/CLI implementation edits.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New whole-board legend asset calls | **0 → 27/27** | **0** |
| Existing Page / SVG / PNG / paint API | **48 / 11 / 18 / 34 → unchanged** | **0** |
| Whole-board SVG / exact source errors | **870 / 138 → unchanged** | **0** |
| Explicit-resource SVG / native PNG boards | **9 / 20 → unchanged** | **0** |
| Rebuilt isometric CLI / e2e SVG+errors | **50 / 933+11 → unchanged** | **0** |

The new `panels` suite compares complete SVG/PNG/Page bytes, annotations and
errors for embedded icons, unconfigured external icons and MaxElements=1.
These are separate source-backed board witnesses, not additional goldens.
Latest public coverage is **93 original calls + 45 board witnesses**. The
permanent fixture preserves requests, exact errors, annotations and byte
hashes/counts. Native contracts also verify source immutability and an explicit
resolver's artwork/limit handoff. No allowance, normalization, exclusion,
tolerance or timeout changed. The first candidate fixture had an unrelated
upstream missing-glyph failure; the qualified original legend-mono boards
retain their own title and pass all Go witness assertions.

**37 native + 25 harness tests pass**; all 37 native tests and all 27 new API
responses repeat after verified compression. Scoped rustfmt/Ruff/gofmt/diff
checks pass. Initial resolver-test URL decoding and guard setup errors were
corrected; logs remain. Hooks are detached and original trace sources restored.
Fresh rendering/API captures use **1 GiB total / 128 MiB per file / 20 GiB
free** streaming guards. The final ordinary e2e replay is also guarded.

Evidence: `iso12-panels-adapter-before.json`, `iso12-panels-final.json`,
`iso12-deltas.json`, existing API/SVG/assets/CLI/e2e guards, native/harness logs,
and `iso12-corpus-check.json`. The audit verifies **939 board inputs, 1,008
SVG/error and 1,008 scene expectations, 504 PNG bindings, nine resource
boards, 138 public calls and 14 latest runner bindings**. Prior latest reports
remain bound in `iso12-latest-retirement.json`; new alias:
`isometric-panels-api.json`. Foundation/language/component tiers are inherited.

Cleanup actually removed **1,018 files / 508.81 MiB allocated path footprint**,
including verified duplicate successful outputs and the disposable baseline
executable. Negative replays and all source/input/expected-output closure
survive (`iso12-cleanup.json`). Applesauce **0.5.30 / verified LZFSE** saved
**173.47 MiB** on 19 idle single-link retained files; hashes, modes, owners
and original xattrs verify, and clone backups are removed. Exact targets and
rollback: `iso12-compression.json`. No bulk moved; path savings do not attribute
concurrent physical-volume/net changes.

Final allocated/apparent GiB: checkout **21.44/35.33**, `.reference` **10.60/22.06**, target **9.45/11.58**; **106.43 GiB free**. Every ceiling passes (`iso12-disk.json`, raw du/df). Full scans include hidden paths/.git and every apparent hard-linked path during concurrent lane activity. No further artifact-heavy work follows this audit.

**Next:** no measured Lane I SVG/API failures remain in these corpora. Shared
SVG-import ownership handoff remains with Lane F, as do GIF/timeline/watch
and CLI work. Synchronous foundation/raster calls still do not provide
in-call cancellation; no broader API or race parity is claimed.

### Lane I iso11: container-gradient admission through public image APIs (2026-10-25)

Read the complete brief through **Update 25**, latest I/F/L handoffs and
STATUS; also read arriving **run59**. Continued Lane I at master
**6a7b48378**. No commits; other lanes' raster, CLI, compiler and language
work and `29` are preserved.

The four next original whole-image paint/icon tests already match on all
**25 public calls**. Static comparison then found a missing validation step
in isometric's local `ParseGradient` adapter. A source-backed witness on
**regression/cylinder_grid_label (ELK)**, using an authored dashed container,
confirms six failures through **SVG, PNG and RenderPage**: an extra stop field
silently disappeared and published three images; a nonfinite position gave
three incorrect errors after rewriting the paint. The adapter now rejects
malformed/empty stops and nonnumeric/nonfinite positions before the opacity
wash, preserving the source index and exact stage error. Valid positions
reuse the existing native Go float formatter. No numeric geometry changed.

Go mapping: **lib/color/gradient.go:27–144**, called by
**gradient_paints.go:194–220** and **hierarchy_boards.go:297–324**.
`crates/d2-rs-isometric/SOURCE.md` records the translation. Production edits
are confined to Lane I's adapter and the existing formatter's visibility.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New whole-image paint API suite | **28 → 34/34** | **0** |
| Existing Page / SVG / PNG API calls | **48 / 11 / 18 → unchanged** | **0** |
| Whole-board SVG / exact source errors | **870 / 138 → unchanged** | **0** |
| Explicit-resource SVG / complete native PNG boards | **9 / 20 → unchanged** | **0** |
| Rebuilt isometric CLI / ordinary e2e SVG+errors | **50 / 933+11 → unchanged** | **0** |

The new suite captures 25 public calls from four original upstream functions,
with every Go assertion active, plus **nine separate board witnesses**
(two malformed gradients and a valid canonical-position control, three APIs
each). Internal helper assertions remain Go-only. Total latest API coverage:
**93 original upstream calls + 18 board witnesses**. Complete image bytes,
page annotations and exact errors compare; no normalization, exclusions or
tolerances changed. Permanent `paint-api.json` retains original requests and
byte hashes/counts, annotations and errors; its native test also checks source
immutability. **34 native + 25 harness tests pass**, including all 20 original
PNG boards. All 34 native tests and new API responses repeat after compression.
Scoped formatting/lint/diff checks pass; inherited Rust warnings remain.

Evidence: `.reference/codex-runs/iso11-paint-admission-before.json`,
`iso11-paint-final.json`, all API/SVG/resource/CLI/e2e guards and deltas,
`iso11-validation.json`, `iso11-corpus-check.json`, and `report-iso11.md`.
New latest alias: **isometric-paint-api.json**. Previous aliases are explicitly
detached with retained prior bytes. Original trace sources are restored and
API hooks removed. Captures use streaming **1 GiB total / 128 MiB per file /
20 GiB free** guards. After cleanup/compression, all **939 original inputs,
1,008 SVG/error and 1,008 scene expectations, 504 PNG bindings, resources,
111 API calls and 13 latest runner bindings** verify. Other tiers are inherited.
Two setup errors (old page-capture name and legacy witness-manifest schema)
were corrected; their logs remain.

**Next:** coordinate the pending shared SVG-import ownership adapter with
Lane F; measure named public resource-limit/cancellation failures before
further API changes. GIF/timeline, watch previews and CLI continuation remain
Lane F's scope. Shared foundation calls remain synchronous; this run does not
qualify cancellation inside those calls or deadline races without a poll.

Cleanup retired **1,024 files / 492.22 MiB allocated path footprint**;
verified Applesauce **0.5.30 LZFSE** saved **156.32 MiB** on 18 idle single-link
files. Hashes/modes/owners/original xattrs verify and clone backups are removed.
Exact paths/rollback: `iso11-cleanup.json`, `iso11{,-cli}-compression-final.json`.
Cargo's regenerated warm executable bindings have an explicit retirement
overlay. Negative replays and input/expected-output closure survive; no bulk
moved or physical-volume/net savings attributed. Final allocated/apparent GiB:
checkout **21.40/35.02**, `.reference` **10.60/22.05**, target **9.36/11.53**;
**110.24 GiB free**. Every ceiling passes (`iso11-disk.json`, raw du/df),
including hidden paths/.git and every apparent hard-linked path during
concurrent lane activity. No further artifact-heavy work follows this audit.

### Lane I iso10: public PNG deadlines and shared page capture (2026-10-25)

Continued the newest handoffs and full brief through Update 25 at master
**6a7b48378**, within Lane I. The next measured failures were public PNG
deadline calls on **stable/all_shapes (ELK), stable/giant_markdown_test and
patterns/paper (Dagre)**: all three published PNGs after a 1 ns timeout.
Added `render_png_with_context` and a shared still-capture path used by Page.
It preserves option/caller/BuildScene admission order, child deadlines,
cooperative mesh and painter checks, typed causes and source stage prefixes.
No numeric geometry, foundation, raster/GIF or CLI implementation changed.
Source mapping: `native.go:24–90`, `render.go:61–112`, `page.go:32–61`, in
the isometric SOURCE.md.

| Tier | Before → final | Regressions |
|---|---:|---:|
| New public PNG/page API capture | **15 → 18/18** | **0** |
| Existing public Page / SVG API calls | **48 / 11 → unchanged** | **0** |
| Whole-board SVG / exact source errors | **870 / 138 → unchanged** | **0** |
| Explicit-resource SVGs | **9 → unchanged** | **0** |
| Complete native PNG fixture boards / isometric CLI cases | **20 / 50 → unchanged** | **0** |
| Rebuilt ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

The new capture observes **12 Render calls from two original upstream test
functions**, with every Go assertion active, plus **six named-board deadline
witnesses** (PNG and Page per board). Internal/CaptureFrames assertions in
those two functions are Go-only and are not counted as native matches.
Complete successful PNG bytes and exact errors compare without normalization;
all input/expected-output inventories and streaming limits remain enforced.
The permanent PNG API fixture retains all 18 requests and exact errors or
PNG hashes/byte counts. The native twenty-board PNG fixture now exercises the
new context entry point. **33 native and 25 harness tests pass**, including
typed mesh-stage cancellation from a resolver callback. All 33 native tests
and the complete 18-call API replay also pass after compression. E2e reuses
Lane L14's freshly built shared CLI and complete guarded run, verifying its
runner hash; no duplicate build/capture. Scoped formatting, Ruff and diff
checks pass; whole-script Ruff's inherited SIM114 is recorded in validation.

Evidence: `.reference/codex-runs/iso10-png-api-{before,final}.json`,
`iso10-deltas.json`, the Page/SVG/explicit-resource/CLI/e2e guards,
`iso10-validation.json`, `iso10-corpus-check.json` and
`iso10-baseline-rebuild.json`. All **939 original inputs, 1,008 SVG/error and
1,008 scene expectations, 504 PNG bindings, resources, 77 API calls and 12
latest runner bindings** verify. Original Go files are restored exactly and
observation hooks detached. Budgets: **1 GiB total / 128 MiB per file /
20 GiB free**. Other lanes and `29` are preserved; no commits.

**Remaining:** blocking foundation resolver/importer/font/raster/encoder
calls remain synchronous. The Lane F-owned `raster::render_png` convenience
path still needs to delegate to `render::render_png_with_context`; GIF,
CaptureFrames and CLI continuation stay in Lane F's scope. General race
precedence without a cooperative poll is not qualified.

Cleanup retired **1,023 files / 545.00 MiB allocated path footprint** of
verified duplicate outputs and superseded Lane I runner copies. Negative
baseline bytes, source inputs and original expected outputs survive.
`iso10-cleanup.json` and `iso10-latest-retirement.json` record exact paths,
hashes, prior alias bytes and replacement/rebuild records. Verified Applesauce
**0.5.30 LZFSE** saved **121.91 MiB** on fifteen idle single-link retained
files; hashes/modes/owners/original xattrs verify and clone backups are removed.
Exact targets/rollback: `iso10{,-native}-compression-final.json`. No bulk
moved; these path savings do not attribute physical-volume/net change.

Final allocated/apparent GiB: checkout **21.01/34.46**, `.reference` **10.38/21.83**, target **9.15/10.95**; **111.77 GiB free**. Every ceiling passes (`iso10-disk.json`, raw du/df). Full scans include hidden paths/.git and all apparent hard-linked paths during concurrent lane activity. No further artifact-heavy work follows this audit.

### Lane I iso9: SVG deadlines and cooperative cancellation (2026-10-25)

Continued the full brief through Update 25, within Lane I ownership. With the
ordinary goldens closed, measured the public SVG API on named existing boards.
`stable/all_shapes` (ELK), `stable/giant_markdown_test` and `patterns/paper`
(Dagre) all returned SVG after an admitted 1 ns timeout. The original SVG
vector, admission and large-document tests add eight already-matching calls.
The capture keeps every original Go assertion active and compares full SVG
bytes; the three deadline witnesses are identified separately from upstream tests.

`render_svg_with_context` now shares the convenience API's path, starts the
admitted timeout, preserves typed caller causes, and polls mesh/painter,
visibility/order/shadow budgets and the SVG writer. Source mappings:
`svg.go:21–108`, `geometry.go:139–155,436–518`, `svg_visibility.go:48–64,343–359`
and the painter/order helpers; recorded in isometric `SOURCE.md`. No numeric
geometry, normalization or tolerance changed. No foundation/raster/GIF/CLI
implementation was edited; foreign work and `29` remain intact. No commits.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Three named-board SVG deadline witnesses | **0 → 3/3** | **0** |
| Original new SVG API calls / existing page API calls | **8/8 / 48/48 → unchanged** | **0** |
| Whole-board SVG / exact source errors | **870 / 138 → unchanged** | **0** |
| Explicit-resource source SVGs / native PNG fixture boards | **9 / 20 → unchanged** | **0** |
| Current CLI isometric source cases / ordinary e2e SVG+errors | **50 / 933+11 → unchanged** | **0** |

**30 native + 63 harness tests pass**, plus the diagnostic mesh example check,
scoped rustfmt/Ruff/Go formatting/diff checks. All native tests rerun from
compressed executables; the compressed CLI passes an isometric smoke. Latest
API captures contain **56 original upstream calls + three deadline witnesses**.
The permanent SVG fixture retains full requests and expected byte counts/hashes.
Existing component/language/foundation tiers are inherited, not newly qualified.

**Open:** cancellation is cooperative; synchronous foundation resolver/importer/
font/raster calls must return before polling can resume. Page/PNG in-stage
contexts and deadline/cancellation races without a poll remain unqualified.
CaptureFrames, GIF/timeline and CLI continuation belong to Lane F. The earlier
SVG-import ownership handoff request remains open.

Evidence: `.reference/codex-runs/iso9-{svg-api-before,svg-api-final,deltas}.json`,
page/SVG/resource/CLI/e2e guards, `iso9-validation.json`, `iso9-corpus-check.json`
and `report-iso9.md`. Capture limits remained **1 GiB total / 128 MiB file /
20 GiB free**. Original source was restored; hooks are detached. Original
inputs/expectations and all eleven current runner bindings verify after cleanup.
Superseded warm/frozen executable bindings have an explicit retirement overlay.

Cleanup actually retired **1,025 files / 548.41 MiB allocated path footprint**;
verified Applesauce **0.5.30 LZFSE** saved **181.55 MiB** on seventeen idle,
single-link retained files. Bytes/modes/owners/original xattrs verify, clone
backups are removed. Exact targets/rollback/deletions: `iso9-compression*.json`,
`iso9-cleanup.json`, `iso9-retirement.json`. No bulk moved; these path savings
are not attributed to physical-volume or concurrent checkout net changes.

Final allocated/apparent GiB: checkout **20.86/34.22**, `.reference` **10.39/21.89**, target **9.00/10.67**; **114.06 GiB free**. Every ceiling passes (`iso9-disk.json`, raw du/df). Scans include hidden paths/.git and all apparent hard-link paths during concurrent lane activity. No further artifact-heavy work follows this audit.

### Lane I iso8: native page export and public metadata contracts (2026-10-25)

Continued the full brief through Update 24 at master **6a7b48378**, within
Lane I ownership. With ordinary SVG and PNG goldens closed, selected the
missing public `RenderPage` API: the five original page test functions and
`TestNativeInteractiveExportsRejectUnsafeLinks`. The original Go assertions
remain active; an env-gated wrapper in the trace checkout captures all **48**
public calls with complete requests, results and original function names.

`img/page.rs` follows `page.go:21–131,285–316`: preflight prettyLink/XML/URL/
connection-hit-area checks, caller metadata budgets, static native PNG plus
fitted dimensions, projected/clipped shape and inline Markdown annotations,
semantic board targets and stable decreasing-area order. `PageError` preserves
typed canceled/deadline causes. `render.rs::normalize` follows
`render.go:61–96` admission order; SVG convenience rendering shares that
validation and the caller's link budget. SOURCE.md records the mapping;
no foundation/raster/GIF/CLI file was edited by this lane.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original public image API calls | **19 → 48/48** | **0** |
| RenderPage calls / complete original test functions | **0 → 29/29; 0 → 6/6** | **0** |
| Whole-board native SVG / exact source errors | **870 / 138 → unchanged** | **0** |
| Explicit-resource source SVGs | **9 → 9/9** | **0** |
| Complete native PNG fixture boards | **20 → 20/20** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

The API tier validates every successful complete PNG and annotation inventory,
compares full encoded bytes and full JSON/errors, and rejects missing/extra
calls, corrupt outputs and failed processes. The three successful original
page exports match all PNG bytes and projected annotations; all 26 original
page error outcomes match. **26 native tests + 62 harness tests pass**
(6 new API gates, 18 existing isometric gates, 38 raster gates), plus the
feature-gated mesh example check, scoped formatting/Ruff/diff checks. The
permanent fixture preserves all 29 original page calls and checks unchanged
input. No normalization, allowance, exclusion or output tolerance changed.
E2e uses the unchanged frozen L6 CLI, which does not link isometric; concurrent
Lane F CLI work and Lane L changes are preserved. No commits by this lane.

Evidence: `.reference/codex-runs/iso8-api-qualified-{before,final}.json`,
`iso8-api-delta.json`, `iso8-{svg,assets}-qualified.json`, all deltas/test logs,
`iso8-corpus-check.json` and `report-iso8.md`. Capture limits remain
**1 GiB total / 128 MiB file / 20 GiB free**. The original trace source is
restored and the API wrapper detached. Source/expectation hashes and current
runner bindings are checked after cleanup. Latest API alias:
`.reference/scoreboard-master-6a7b4837/isometric-api.json`.

**Open:** interruption inside synchronous mesh/raster/encoding stages is not
implemented by the page context API; it polls metadata/projection loops and
between stages. Full renderer context/timeout parity, remaining public API
and original inline tests need named source-backed failures. GIF/timeline and
CLI wiring belong to Lane F; the SVG-import ownership handoff request from
iso7 remains with that lane. Static page matches do not close these gaps.

Cleanup/compression ledgers and final whole-checkout disk audit are recorded
in `iso8-cleanup.json`, `iso8-compression.json` and `iso8-disk.json`.

Final allocated/apparent GiB: checkout **20.21/33.46**, `.reference` **10.23/21.63**, target **8.59/10.20**; **28.50 GiB free**. Every ceiling passes (`iso8-disk.json`, raw du/df). Full scans count hidden paths and every apparent hard-link path, during concurrent lane activity. Cleanup retired **1,027 files / 509.29 MiB allocated path footprint**; verified compression saved **33.09 MiB**. No bulk moved or artifact-heavy work after this audit.

### Lane I iso7: bounded icons and explicit whole-board assets (2026-10-25)

Continued the complete brief through Update 22 and newest lane reports at
master **6a7b48378**. Selected all **87** remaining isometric SVG-tier error
mismatches, then the named resource boards `stable/images`,
`regression/grid_image_label_position`, and `txtar/sql-table-reserved`.
Stayed in Lane I ownership; concurrent foundation raster/CLI and language
changes are preserved. No commits by this lane.

`img/surface_icons.rs` now translates `surface_icons.go:23–301`: explicit
external-resolver admission, bounded data-URI defaults, source/content/texture
identity caches, caller-tightened SVG import limits and cumulative budgets,
aspect fit, rounded clips, native rasterization and retained vector documents.
`Options.assets` and `geometry.go:436–477` icon counts/density connect the
painter to whole-board rendering. Parsing, resource acquisition, first-frame
normalization and rendering execute in native foundation crates. Their private
SVG arena ownership bridge has a thin local adapter in `img/backends.rs`;
exposing that bridge would remove the adapter. SOURCE.md maps the port.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Whole-board golden SVGs | **870 → 870/870** | **0** |
| Exact source error outcomes | **51 → 138/138** | **0** |
| Original SVG-tier mismatches | **87 → 0** | **0** |
| Complete explicit-asset source SVGs, D/E/T | **0 → 9/9** | **0** |
| BuildScene outcomes | **1008 → 1008/1008** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

Every ordinary isometric outcome is accounted for: **870 SVGs + 138 errors =
1008**. Explicit assets are a separate profile of the same whole-board SVG
tier: unchanged original locators, immutable hash-bound original SVG/JPEG
resources and an injected transport with no network fallback. These nine
`source-match` results do **not** claim additional upstream golden matches;
the unconfigured upstream tests deliberately expect external-source errors.
The harness checks inventory, exact complete bytes, SVG validity, input hashes
and process/budget completion. No allowance, normalization or exclusion changed.

**19 Lane I native tests + 18 harness tests pass**; the same native run also
passes three concurrent Lane F raster tests. Three permanent whole-board
fixtures retain their source resources, independently reproduced Go SVGs and
exact unconfigured errors. Go hooks compile, the diagnostic mesh example
checks, and scoped formatting/diff checks pass. Text/rich/primitives/writer
corpora remain inherited regression evidence; they were not recaptured.
The startup e2e guard uses the frozen L6 CLI, which does not link isometric.
No broad image-format, context/timeout, GIF/timeline or CLI qualification is
inferred. PNG and CLI work belong to Lane F under Update 22.

Evidence: `.reference/codex-runs/iso7-{svg,assets,e2e}-delta.json`,
`iso7-svg-initial.json`, `iso7-scene-qualified.json`,
`iso7-assets-source/qualified.json`, tests and `report-iso7.md`.
Latest SVG/scene aliases and the explicit-assets SVG profile bind the current
frozen probe. After cleanup/compression, **939 inputs, 1008 source SVG/error
expectations, 870 goldens, 1008 scenes, nine explicit-resource SVGs and all
current runner hashes verify** (`iso7-corpus-check.json`). Diagnostic argument
construction remains behind `diagnostic-traces`.

Cleanup retired verified successful replay outputs and one superseded frozen
probe, preserving source expectations, inputs, negative outputs and rebuild
records. `iso7-cleanup.json`, `iso7-runner-retirement.json` and
`iso7-compression-smoke.json` are the deletion ledgers. An initial cleanup
precondition caught scene JSON encodings that compare equal semantically but
have different byte hashes; deleted SVGs were first reinstated by exact hash,
then the persistent-ledger cleanup completed (`iso7-cleanup-reconstruction.json`).
No input/expected-output closure was deleted. Applesauce **0.5.30 / LZFSE**
verified 14 idle, single-link retained artifacts, saving **28.63 MiB**, with
bytes/modes/owners/original xattrs preserved and backups removed. Both compressed
probes reproduce the whole-board asset smoke SVG. Capture limits remained
**1 GiB total / 128 MiB file / 20 GiB free**. Final savings and fresh whole-
checkout allocated/apparent/free measurements are recorded in `iso7-disk.json`.

Next Lane I work must start from a named failing whole-board SVG/API outcome;
the current ordinary corpus has none. Broader asset/context APIs require new
source-backed failing witnesses. Do not repeat the closed error cluster or
expand component qualification tiers.

Final allocated/apparent GiB: checkout **19.83/32.89**, `.reference` **10.03/21.42**, `target` **8.30/9.84**; **28.15 GiB free**. Every ceiling passes (`iso7-disk.json`, raw du/df). Cleanup actually retired **2,036 files / 527.61 MiB allocated path footprint**, plus **28.63 MiB** verified transparent-compression savings. Full scans include hidden paths/.git and every apparent hard-link path, during concurrent lane activity; gross path savings do not attribute volume/net checkout changes. No artifact-heavy work followed this audit.

Final allocated/apparent GiB: checkout **19.77/32.89**, `.reference` **10.03/21.42**, `target` **8.30/9.84**; **28.15 GiB free**. Every ceiling passes (`run56-disk.json`, raw du/df). Whole-checkout scans include hidden paths and every apparent hard-link path; measurements include concurrent lane activity. No artifact-heavy work follows this audit.

### Lane I iso6: complete routed whole-board SVG goldens (2026-10-25)

Continued the full brief through Update 21 and newest lane handoffs within
Update 15's Lane I scope at master **6a7b48378**. Selected the **556** routed
SVG failures, starting with `patterns/shape` and `real_world/fulcro_rad`.
The mesh assembly now translates `route_lanes.go`, `route_bridges.go`,
`geometry_routes.go`, `arrow_visibility.go`, `route_style.go`,
`route_caption_style.go`, `route_caption_mask.go` and `sequence_routes.go`.
It preserves source ports, independent lane/bridge/contact paths, flat ink
and casing, bounded visibility/cache work, authored marker shapes, compiled
caption anchors and mask geometry, sequence partitioning, opacity, links and
animated packet framing. Edge rich-label counts and completed arrow-owner
bindings follow `geometry.go:450–465,599,616–627`.

The first full native run matched **823/870**. All remaining **47** SVG
mismatches differed solely in hollow-marker knockout color. Per-board mesh
inspection targets `stable/box_arrowhead`; static source comparison found
`geometry.go:482` initializes from **d2target.BG_COLOR = N7**, while the
inherited assembler used B6. The generic correction closes the cluster.
No comparator normalization, fixture-specific output, exclusion or allowance
changed. Four complete route meshes/documents (`patterns/shape` Dagre and
`real_world/fulcro_rad` D/E/T) match, with external font references resolved
losslessly to their exact bytes. Fresh pinned ARM64 instruction excerpts map
the fused arithmetic. Source mapping: isometric `SOURCE.md`.

| Tier | Before → qualified current | Regressions |
|---|---:|---:|
| Whole-board native SVG | **314 → 823 → 870/870** | **0** |
| Source resource errors | **18 → 51/138** | **0** |
| BuildScene / full writer on Go meshes | **1008 / 870 → unchanged** | **0** |
| Plain nonempty boards / exact calls | **762 / 9988 → unchanged** | **0** |
| Rich nonempty boards / exact calls | **192 / 438 → unchanged** | **0** |
| Primitive nonempty boards / exact calls | **861 / 22585 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All **1008** outcomes are inventoried. Native SVG matches: Dagre **298**,
ELK **286**, TALA **286**. The remaining **87** outcomes are explicit source
resource-error mismatches, principally the unported icon painter; they are
not SVG matches. The CLI isometric flag/help, PNG/GIF/timeline and general
context/asset APIs remain open. This qualifies compiled-diagram static SVG
rendering for this corpus, not all isometric APIs or full D2 parity.

**16 native tests and 16 harness checks pass**. Permanent whole-SVG fixtures
now include `patterns/shape` and `stable/box_arrowhead`, replacing the obsolete
assertion that routed boards must fail. Scoped rustfmt and diff checks pass.
Normal builds still omit all diagnostic mesh argument construction; the
separate mesh example requires **diagnostic-traces**. No new component tier.
The e2e guard uses the unchanged frozen L6 CLI, which does not link isometric.
All existing painter/tessellation guards have fresh complete Go captures and
retain every prior match. All six latest master isometric reports now bind
five frozen current executables; original hashes verify **939** pristine board
inputs, **1008** SVG/error expectations, **870** SVG goldens, **1008** scenes,
six latest runner bindings and all five mesh witnesses. The marker's negative
and corrected mesh are retained. Source bodies match the pristine pin after
instrumentation. `iso6-corpus-check.json`, `iso6-validation.json` and
`report-iso6.md` record the checkpoint. Prior runner/alias retirements are
explicit in `iso6-runner-retirement.json`.

Evidence: `.reference/codex-runs/iso6-svg-{initial,final,delta}.json`,
`iso6-marker-delta.json`, `iso6-scene-qualified.json`,
`iso6-surfaces-final.json`, mesh witnesses, native/harness logs and compiler
instruction excerpts. Captures retain **1 GiB total / 128 MiB file / 20 GiB
free** streaming guards. Every reused independent Go output and each of the
870 source goldens was reverified before native comparison. No commits;
foreign work and `29` preserved.

Cleanup (`iso6-cleanup.json`) actually retired **2,811** obsolete/verified
redundant files, **1035.09 MiB allocated path footprint**, plus **10,440**
streamed successful outputs (**4.11 GiB path footprint**) and **16,512** empty
directories. Original negative SVG/error witnesses, source inputs and expected
outputs remain. The full compiler dump has compact instruction replacements
and its rebuild command. Five superseded positive iso5 executables were idle
and verified before retirement. No bulk moved. These are separate gross path
figures, not physical-volume or concurrent-checkout net savings. No per-run
Applesauce pass: final `.reference` allocation is below the brief's **10 GiB**
trigger.

Final allocated/apparent GiB: checkout **19.41/32.32**, `.reference`
**9.94/21.21**, `target` **8.03/9.48**; **28.99 GiB free**.
Every ceiling passes (`iso6-disk.json`, raw du/df). Whole-checkout scans include
hidden paths/.git and every apparent hard-link path; concurrent lanes remain
active. No artifact-heavy work followed this audit.

### Lane I iso5: first whole-board native isometric SVGs (2026-10-25)

Read the full brief through **Update 21**, the newest handoffs and STATUS;
continued within Update 15's Lane I scope at master **6a7b48378**. Activated
native node/face, solid, class/SQL row, hierarchy/header, sequence-node, link,
paint-owner, ground-receiver and root-decoration assembly, connecting it to
`img::render::render_svg`. Shared retained documents now support apertures
and ink coverage with their original identities. Foundation calls stay in
thin `img/backends.rs` adapters; foundation crates are unchanged by this lane.

The first complete SVG run scored **296/870**. Its 18 actual SVG mismatches
were root frames/legends, including `root/stroke-width`, `root/border-radius`
and `txtar/legend-mono`. Per-board Go/native mesh comparison found the first
divergence: the invisible measuring shape omitted Go's **BaseShape stroke
width 2** (`d2target.go:886`, `source_panels.go:117–125`). The panel also must
retain the raster helper's final logical dimensions and centered meet viewport
(`surface_document.go:123–152`). Those shared fixes close all 18 boards;
the two complete witness meshes/documents now match exactly. ARM64 instruction
witnesses record the first-product background FMA and structured placement
sites. Source mapping is in isometric `SOURCE.md`.

| Lane I tier | Before → qualified final | Regressions |
|---|---:|---:|
| Native whole-board SVG, complete bytes | **0 → 296 → 314 / 870** | **0** |
| Scene / complete writer on Go meshes | **1008 / 870 → unchanged** | **0** |
| Plain text, nonempty boards / calls | **762 / 9988 → unchanged** | **0** |
| Rich labels, nonempty boards / calls | **192 / 438 → unchanged** | **0** |
| Primitives, nonempty boards / calls | **861 / 22585 → unchanged** | **0** |
| Ordinary e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

Whole SVG matches: Dagre **107**, ELK **103**, TALA **104**. Every one of
**1008** source boards is inventoried. Remaining: **556 SVG boards explicitly
fail on unported route assembly**; 18 source resource errors match and 120
still differ. These source errors do not count as SVG golden matches.
The CLI isometric flag, PNG/GIF/timeline and general context/asset APIs remain
open. Other language/export/raster tiers are inherited; the ordinary guard
uses an unchanged frozen L6 CLI which does not link isometric.

All **16 native tests + 16 harness tests** pass; new-module rustfmt, Ruff and
scoped diff checks pass. The separate `isometric-mesh` debugger requires
**diagnostic-traces** at Cargo target selection; the default probe contains
no mesh-dump argument construction. No new component qualification tier was
added. The SVG harness now rejects missing/duplicate/extra/empty outputs,
stale Go goldens and failed producers; Finder metadata is explicitly recorded.
A real one-byte capture fails with zero matches and reaps Go. Limits stay
**1 GiB total / 128 MiB file / 20 GiB free**. Two free-space-interrupted runs
remain failures; fresh whole-checkout audits passed before their successful
retries. A stale scene-runner binding caught at retirement was replaced by a
fresh **1008/1008** replay on the current default probe, then reverified.

Evidence: `.reference/codex-runs/iso5-svg-{before-inventory,qualified-initial,
final,qualified}.json`, `iso5-panels-delta.json`, all six tier final/qualified
reports and deltas, `iso5-mesh-debug`, native/harness logs,
`iso5-validation.json` and `iso5-corpus-check.json`. SVG/scene final default
probe replays verify all original independently captured Go hashes; all 870
SVG golden bindings are rechecked. Rich replays all 192 retained original
nonempty call inputs; its empty/source-error inventory is inherited. Scene,
writer, primitives and text have completed fresh Go captures. Rich aggregate
counters were recomputed from actual rows, correcting the inherited stale
168/402 counter without changing any request, comparison or match.
Latest master isometric reports are updated. No commits; foreign work and
`29` are preserved.

**Next by board impact:** connect the routed-edge pipeline for those 556
boards (e.g. `patterns/shape`, `real_world/fulcro_rad`): `geometry_routes`,
`route_lanes`, `route_bridges`, endpoint/arrow visibility, caption style/masks
and sequence routes. Keep complete SVG as the progress gate and use per-board
mesh dumps for the earliest divergence. Icons and remaining resource-error
contracts follow. Optional foundation reuse: expose the private icon-size
and raw-gradient parsing helpers to retire their small local adapters.

Cleanup and final disk figures are recorded in `report-iso5.md` and
`iso5-cleanup.json` / `iso5-disk.json`; current source inputs, expected outputs,
frozen runner hashes and four restored trace source bodies verify after cleanup.

Cleanup retired **3,544 obsolete files**, **1908.86 MiB allocated path footprint**; the scoped latest capture directories fell by **886.04 MiB**. Also retired **10,164 streamed outputs**, **4.18 GiB path footprint**, and **15,806 empty directories**. These are separate from physical-volume changes and concurrent cleanup. No bulk moved. `.reference` stays below the brief's 10 GiB compression trigger, so no per-run compression pass.

Final iso5 disk: checkout **18.21/30.76 GiB** allocated/apparent, `.reference` **9.76/20.91**, `target` **7.40/8.39**; **30.90 GiB free**. Every ceiling passes; no further artifact-heavy run followed (`iso5-disk.json`).

### Lane I iso4: active native mesh tessellation (2026-10-25)

Continued the full brief through Update 19 and latest handoffs, including
run 50's completed rich-label prerequisite. Chose the common native mesh
blocker on `real_world/fulcro_rad`, `regression/grid_oom` and
`txtar/border-label`, across engines. Extracted the existing gated tessellation
into active `geometry_core.rs`: bounded triangle appends, rounded cuboids,
spheres/cylinders and profile extrusion with winding, smooth convex wall
normals and ordered ear clipping. The higher-level gated builder delegates
to that core; its duplicate tessellation code is removed.

The new `--tier primitives` observes actual outermost Go mesh operations,
then compares every ordered triangle, vertex/normal/UV, material value and
identity, shadow flag and error. Go must independently reproduce the whole
SVG golden first. All **1008 boards** remain accounted for: **861 nonempty
boards / 22585 calls match**, nine explicit empty inventories and 138 Go
resource errors. Actual calls are **22576 profile extrusions and nine rounded
boxes** (`txtar/border-label`, all engines). Nested flat faces are compared
within extrusion outputs; static SVG boards do not exercise sphere/cylinder
outputs. Initial/final sweeps have **zero changed inputs, changed outputs or
regressions**. This is the first active tessellation qualification, **not a
whole-board SVG gain**: native complete meshes/end-to-end SVG remain **0/870**.

Source mapping: `geometry.go:140–276`, `native_shapes.go:642–747`, isometric
SOURCE.md. The pinned ARM64 assembly confirms sphere coordinate FMADDD and
second-product FMSUBD for signed area/turn/ear tests; compact instruction
witnesses survive. Original source bodies are restored byte-exactly after
capture. Actual rectangle/cap and bevel-box calls are permanent fixtures.
**14 native tests and 11 harness checks pass**, with the final two-operation
fixture rerun separately. The real one-byte capture fails with zero matches,
12 missing boards and a reaped producer; both trace source files restore
exactly. Ruff, scoped rustfmt and diff checks pass. Ordinary e2e remains
**933 SVG + 11 expected errors**, **zero regressions**. The CLI guard uses
frozen independently built executables and does not link isometric yet.

Evidence: `.reference/codex-runs/iso4-primitives-{initial,final,delta}.json`,
e2e before/final/delta, native/harness/format logs, budget failure, instruction
witnesses and `isometric-primitives.json` in the master scoreboard directory.
The freshly rebuilt complete writer also remains **870/870**, with **zero
regressions**, and all **2610** source SVG bindings verify after cleanup.
Context/error paths beyond the measured append
contracts, animation and complete mesh assembly remain unqualified.

Cleanup retired **154 obsolete files / 130.34 MiB allocated path footprint**,
**7866 streamed outputs / 3.62 GiB footprint**, and 9995 empty directories.
`iso4-cleanup.json` records exact replacements and executable rebuilds;
original negative bytes survive. A late threshold audit put `.reference`
above 10 GiB: obsolete artifacts were removed first, then one verified
Applesauce LZFSE pass on three idle retained queue_workers meshes saved
**25.89 MiB**. Checksums, modes, flags and non-compression xattrs validate;
all clone backups were immediately removed. No bulk moved. Gross path
retirement/compression savings are separate from physical free-space changes.

Final allocated/apparent GiB: checkout **21.07/32.37**, `.reference` **9.68/20.83**, `target` **9.92/9.92**; **33.95 GiB free**. Every ceiling passes (`iso4-disk.json`, raw du/df). These are fresh post-cleanup scans during concurrent lane writes; no artifact-heavy run followed. Handoff: `report-iso4.md`.

Next: integrate the shared mutable aperture/coverage registry and native face
construction, then finish hierarchy/route/structured dependencies and add the
**complete mesh** tier before native end-to-end SVG. Keep the tessellation,
painter and full writer guards. Lane F's run 50 now qualifies all 192 rich
boards; do not repeat its LaTeX work. No foundation/language changes or commits
by this run; foreign work and `29` preserved.

### Lane I iso3: rich surfaces and bundled text fallback (2026-10-25)

Continued the full brief through Update 19 and the latest lane handoffs.
Ported the rich-label painter's minimal target, source/row/pixel budgets,
theme/family selection, physical viewport fitting, raster composition and
retained surface. The new `--tier rich` captures actual source mesh-builder
calls, independently reproduces each Go SVG, and compares the complete
retained SVG, viewport and premultiplied pixel hash. All 1008 boards remain
inventoried: 192 with rich calls, 678 explicit empty inventories and 138 Go
errors. Class/SQL cells bypass this painter and are not qualified by this tier.

The shared viewport bug was applying aspect-ratio fitting after normalization.
`vector_surface.go` fits into the physical viewport first and normalizes the
result. Native scene construction and raster pixels already agreed; preserving
that order fixes the retained surface transform. Integrated Lane F's bounded
bundled fallback resolver for `surface_fonts.go`, including strict glyph
coverage, ink bounds/alignment and two RGBA composition steps. All three
`unicode/emojis/{dagre,elk,tala}` boards now match. Replaced the local static
PNG encoder with the shared first-frame normalization API; the seven original
Go PNG contracts still pass. No foundation or language source was edited.

| Measured tier | Before → after | Regressions |
|---|---:|---:|
| rich-label nonempty boards | **0/192 → 168/192** | **0** |
| exact rich-label calls | **0/438 → 402/438** | **0** |
| plain-label nonempty boards | **759/762 → 762/762** | **0** |
| exact plain-label calls | **9979/9988 → 9988/9988** | **0** |
| complete SVG writer on Go meshes/documents | **870 → 870** | **0** |
| ordinary e2e SVG / expected errors | **933 / 11 → 933 / 11** | **0** |

All before/after painter input hashes are unchanged. The remaining 24 rich
boards (36 calls) require Lane F's LaTeX scene/SVG-import path. Examples fixed:
`real_world/fulcro_rad`, `regression/no-lexer`, `regression/markdown_emoji_cutoff_2680`,
`stable/md_code_block_indented` and `txtar/dark-theme-md` (full list in
`iso3-rich-delta.json`). Native mesh construction and end-to-end isometric SVG
remain **0/870**, pending missing mesh modules and mutable aperture/coverage
registry integration. No full renderer or structured-cell claim follows from
these painter results. Scene/language/CLI-export qualification is inherited.

Source mapping: isometric `SOURCE.md`, rich_labels.go, surface_fonts.go,
vector_surface.go and vector_surface_svg.go. fitRichViewport's half scaling
is exact; the pinned ARM64 inventory supplies synthetic italic-bound FMSUBD /
FMADDD sites at surface_fonts.go:248–249 and readableSurfaceInk contractions.
**11 native tests and 9 harness checks pass**, including actual code/Markdown
and emoji fixtures. Ruff and scoped diff checks pass. The real one-byte rich
capture fails with zero matches/12 missing rows, reaps its producer and restores
both source painter files exactly. The complete writer remains **870/870**;
all 2610 source SVG bindings across rich/text/writer runs verify against
pristine goldens. Capture peak/largest MiB: rich **14.23/13.91**, text
**19.12/13.91**, writer **75.80/49.20**, under unchanged 1 GiB / 128 MiB /
20 GiB-free guards. Evidence: `.reference/codex-runs/iso3-{rich,text,surfaces}-final.json`,
per-tier deltas, e2e before/final/delta and `iso3-validation.json`; latest
painter/writer reports copied to the master scoreboard directory.

Cleanup: **241 obsolete files**, **125.05 MiB allocated path footprint**,
plus **10,008 verified streamed files** (**3.91 GiB path footprint**) and
12,652 empty directories. Direct own-capture du reduction: **18.87 MiB**.
`iso3-cleanup.json` links the deletion/streaming ledgers and exact font/output
replacements. Original painter negatives, all before-call inputs, current
LaTeX witnesses and one initial wrong-viewport witness remain. No older
input/output closure was removed and no bulk moved. Gross retired footprints
are separate from physical free space and concurrent checkout growth.
Shared target overages paused new parity runs until cleanup and fresh whole
allocated/apparent audits passed. Final allocated/apparent GiB: checkout
**20.96/32.20**, `.reference` **9.68/20.77**, `target` **9.80/9.76**;
**26.70 GiB free**. All ceilings pass (`iso3-disk.json`; target/free refreshed
after the parallel whole-checkout scans). Measurements include concurrent
lane changes. Compression deferred per Update 3 (.reference below 10 GiB).
Handoff: `report-iso3.md`. No commits by this run.

### Lane I iso2: native plain-label surfaces (2026-10-25)

Replaced the gated plain-text painter stub with native bounded shaping,
physical fitting, float32 glyph outlines, underline/background paint and
premultiplied texture rasterization. The painter now retains its own scene
surfaces through an export-scoped texture-identity registry. This is a
prerequisite for the mesh builder; the other mesh modules stay gated.

New `isometric-scoreboard.py --tier text` captures actual painter calls from
Go's mesh builder, independently reproduces the full Go SVG golden, then
compares native layout, dimensions, every premultiplied RGBA byte by SHA-256,
and complete retained surface SVG bytes. All **1008 boards** are inventoried:
**762 have plain calls**, **108 have empty plain-call inventories**, and
**138 Go resource errors remain non-passing**. Empty inventories are visible
and do not establish painter coverage.

The initial native port matched 729/762 nonempty boards. Thirty boards
(`regression/just-width`, `regression/disclaimer`, long arrowhead/sequence
labels and others listed in `iso2-text-delta.json`) exposed x/image/vector
curve arithmetic. Pinned Go 1.27 ARM64 assembly fuses every de Casteljau
interpolation (`vector.go:52`) and the second squared deviation (`:260`),
while its line integrators explicitly round their products. Ported those
sites rather than changing the comparator. Static source mapping is in the
isometric SOURCE.md; original negatives and three checked-in Go board
witnesses remain.

| Measured tier | before → after | regressions |
|---|---:|---:|
| plain-label boards with calls | **729/762 → 759/762** | **0** |
| exact plain-label calls | **9947/9988 → 9979/9988** | **0** |
| complete SVG writer on Go meshes/documents | **870 → 870** | **0** |
| isometric BuildScene | **1008 → 1008** | **0** |
| ordinary e2e SVG / expected errors | **933 / 11 → 933 / 11** | **0** |

The text-tier board delta including the 108 explicit empty inventories is
837 → 867 matches; the other rows are 3 mismatches and 138 Go errors. Every
before/after text input hash is unchanged. The three mismatches are
`unicode/emojis/{dagre,elk,tala}`: nine calls require the unported
surface_fonts.go fallback path. Foundation API request: a real bundled font
fallback resolver for d2scenebuild (its public resolver boolean currently
returns unsupported). No foundation or language source was edited. Cargo
updated only this crate's generated dependency entry in the shared lockfile;
other lanes' lock changes are preserved.

**Native mesh construction and end-to-end isometric SVG remain unported
(0/870); this does not qualify rich labels, icons, custom fonts or PNG/GIF.**
Next: finish the missing mesh dependencies and mutable aperture/coverage
registry, add the mesh tier, then wire native SVG. Preserve this painter and
the complete surface writer as regression guards.

Validation: **7 native tests + 7 harness tests pass**; focused rustfmt,
Ruff and diff checks pass. The actual 1-byte-budget negative exits nonzero,
reports 12 missing rows and zero matches, reaps its producer and restores
trace text.go exactly. Streaming guards now also monitor native subprocess
writes. Text capture sampled peak **23.92 MiB** / largest file **13.91 MiB**;
full writer **75.80 / 49.20 MiB**, under unchanged 1 GiB / 128 MiB limits.
All successful writer SVG hashes equal pristine goldens. Latest reports:
`.reference/scoreboard-master-6a7b4837/isometric-{text,surfaces,scene}.json`.
Detailed evidence: `iso2-{text,surfaces,scene}-{final,delta}.json`, e2e
before/final/delta, `iso2-validation.json` and `report-iso2.md` in codex-runs.
CLI/export/foundation API scoreboards are inherited, not new iso2 measurements.

Cleanup: **9,865 files retired**, **3.05 GiB allocated path footprint**
(**3.04 GiB apparent**), including streamed passing outputs,
verified scene duplicates, obsolete replays and assembly. A separate scoped
du check measured **55.05 MiB** reduction during final capture cleanup.
The ledger preserves all exact replacement hashes, initial negatives and
final emoji witnesses; no older input/output closure was deleted. Gross
retirement is separate from net checkout growth and physical-volume free
space. The 2,580 initial-capture events retain exact hashes but no allocation measurements and are excluded from the measured allocation total. Compression deferred per Update 3 (.reference below 10 GiB).
Final allocated/apparent GiB: checkout **20.46/31.70**, `.reference` **9.34/20.42**, `target` **9.69/9.66**; **35.74 GiB free**.
Every ceiling passes (`iso2-disk.json`); measurements include concurrent lanes.

### Lane I iso1: complete retained-surface SVG serialization (2026-10-25)

Ported `vector_surface_svg.go`, `vector_surface_text.go`,
`vector_surface_color.go` and the time-zero animation snapshot. Geometry,
paint servers, clips/masks/filters, shaped glyph outlines and reuse,
COLRv0/v1, decorations and retained ink coverage now serialize natively.
Shortest float formatting must use Go's ties-to-even digits: Rust's
shortest formatter broke all 12 `sanity/*` SVGs on float32-derived glyph
coordinates. The pinned ARM64 assembly confirms colorPaint fuses the first
product at Go lines 82/87 and subtracts fused products at line 88.

The other shared failure was the built-in paletted grain PNG in
`patterns/real-lines/dagre` and `real_world/queue_workers/{dagre,elk,tala}`.
A local native static-PNG adapter reproduces Go's decoded image type,
palette depth/transparency, row filters, default zlib and IDAT chunking.
Seven small Go-generated contracts cover palette depths, transparent
palettes, opaque/alpha NRGBA and Gray16. Other authored raster formats and
animated PNG remain explicit API gaps. Source mapping: isometric `SOURCE.md`.

| Measured tier | before → after | regressions |
|---|---:|---:|
| complete SVG on Go mesh/document inputs | **2/870 → 870/870** | **0** |
| surface-stubbed writer | 870 → 870 | 0 |
| isometric BuildScene | 1008 → 1008 | 0 |
| e2e SVG / expected errors | 933 / 11 → 933 / 11 | 0 |

All **1008** writer rows are inventoried; **138 Go resource errors are
non-passing**. Go independently reproduced each of the 870 full SVG goldens
before native comparison. The final run uses one frozen executable; all
870 source/native SHA-256 bindings verify against pristine goldens. The
complete-output gain is a **writer/surface tier**, not native mesh or
end-to-end isometric parity: the SVG end-to-end tier remains **0/870**.
The unfinished builder stays gated. Foundation and unrelated language files
were not edited by Lane I. CLI/export/raster foundation scoreboards were
not rerun; their inherited results are not new measurements.

Harness: new `--tier surfaces`, explicit input-derived inventory, atomic
publication and one-mesh producer/consumer acknowledgment. Missing outputs,
failed producers, native timeouts and budget exhaustion cannot pass. A
forced 1-byte-budget negative exits nonzero with `go-missing`, zero matches,
and no surviving Go producer. The first aggregate attempt exceeded 1 GiB;
it remains a failed run and its finished bulk was retired with a ledger.
Final capture peak **75.81 MiB**, largest file **49.20 MiB**, under unchanged
1 GiB / 128 MiB / 20 GiB free limits. Completed source/native SVGs are
retired after exact hash binding; old stub negatives are reconstructible
from `stub_surfaces(golden)` with their original hashes. Font inputs are
verified and deduplicated, preserving old witness DataFile paths.

Validation: **5 native tests** (including 7 PNG oracle records), **4 harness
tests**, focused Ruff and diff checks pass. Evidence under
`.reference/codex-runs/`: `iso1-surfaces-{before,final,delta}.json`,
`iso1-scene-{final,delta}.json`, `iso1-e2e-{before,final}.json`,
`iso1-e2e-delta.txt`, guard/test logs and cleanup ledgers. The unused
`before_summary` in the final raw capture has no baseline writer; use the
separate `iso1-surfaces-before.json` and delta for the measured baseline.

Next Lane I work: finish/enable the mesh builder's missing dependencies,
reconcile its mutable vector registry with the qualified surface serializer,
then add the mesh comparison tier and run end-to-end SVG. Keep the new full
writer tier as a regression gate. A shared foundation API for first-frame
raster normalization/default PNG encoding would replace the local adapter;
Lane I made no foundation changes. CLI `--isometric` follows native rendering.

Cleanup: **2,210 retired files/events**, **1.61 GiB allocated / 1.60 GiB apparent** recovered, with five deletion ledgers referenced by `iso1-cleanup.json`. Only iso1 obsolete captures, replays, verified font duplicates and compiler assembly were removed; current inputs, original negatives and final evidence remain. No bulk moved. Compression deferred per Update 3 (.reference below 10 GiB allocated). Final allocated/apparent GiB: checkout **20.29/31.66**, `.reference` **9.30/20.38**, `target` **9.70/9.61**; **38.53 GiB free**. Every ceiling passes (`iso1-disk.json`, sealed raw du/df). Gross iso1 cleanup savings are separate from concurrent checkout growth.

### Run 55: bounded shared image-resource cache (2026-10-25)

Read the newest reports/messages, STATUS and the complete brief through
Update 21. Continued Lane F at master **6a7b48378**; other lanes and `29`
are preserved. Selected the absent shared-cache contract supporting the named
asset-bearing boards `stable/images`, `regression/grid_image_label_position`
and `txtar/sql-table-reserved`. This is a foundation prerequisite; **no new
whole isometric-board or pixel match** is claimed.

Static comparison mapped the fix to `lib/imageasset/imageasset.go:215–302,
348–354,389–396`, `resolve.go:107–159,633–682` and `lib/localfile/localfile.go:
111–132`. The caller-owned `ImageMemoryCache` enforces inclusive entry/encoded
byte bounds with deterministic LRU eviction and immutable snapshots. Namespace
and explicit transport-policy tags isolate cache keys. File readers admit
policy-scoped canonical keys before lookup; custom readers skip shared file
caching by default. Shared hits revalidate all per-resource budgets before
atomically charging each resolver's own cumulative capacity. Same-session hits
charge once, data URIs stay out of the shared cache, and failed/canceled calls
do not publish resources. Source mapping is in scenebuild SOURCE.md.

| Tier | Before → final | Regressions |
|---|---:|---:|
| Original HTTP cache eviction/limit calls | **0 → 11/11** | **0** |
| Original inline/local/base-directory cache calls | **2 → 11/11** | **0** |
| Original tenant-namespace calls | **0 → 3/3** | **0** |
| Existing original raster/SVG/codecs/HTTP/header calls | **50 → 50/50** | **0** |
| Asset / LaTeX scenes | **28 / 8 → unchanged** | **0** |
| Ordinary scene outcomes | **1008 → 1008** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

The 25 new calls come from seven original test functions with all Go
assertions active. The pre-implementation adapter explicitly rejects the absent
shared-cache API; the two uncached calls execute normally. Replay compares full
resources, errors, session charges and cache occupancy. Captured opaque namespace
prefixes preserve the original equivalence classes; no expected value or input
is rewritten. The direct empty-namespace constructor assertion is explicitly
uncaptured, with its exact native rejection independently unit tested.
**44 native tests + four harness checks pass**. Additional native contracts
cover returned snapshot ownership, no-I/O hits after file removal, custom-reader
opt-in, namespace/policy/base isolation, canceled hits, atomic failure/retry and
concurrent seeded hits with independent session charges. Scoped rustfmt, Ruff
and diff checks pass. No comparison allowance, exclusion, normalization, FMA or
diagnostic trace construction changed. Other CLI/raster/language/isometric
tiers are inherited; these finite calls do not establish general API parity.

Evidence: `.reference/codex-runs/run55-{cache,cache-local,cache-namespace}-
{before,final,delta}.json`, all five acquisition guard reports/deltas,
scene/asset/LaTeX/e2e reports/deltas, native/harness logs and
`run55-validation.json`. Permanent fixtures retain every new request/result.
All 939 scene inputs, 1008 original expectations, nine font hashes, 75 original
resource calls and five retained executable hashes verify after cleanup
(`run55-corpus-check.json`). Trace bodies are restored exactly and the hook
removed. Latest foundation aliases are updated. Captures retain **1 GiB total /
128 MiB file / 20 GiB free** guards; current scene/e2e streaming guards pass.

Open: rooted local-file policy, custom cache adapters, full concurrent loader/
context/transport ownership and malformed URL/codec contracts. Isometric asset
integration/CLI wiring stays with Lane I. No commits.

Cleanup retired **1,047 files / 143.86 MiB allocated path footprint**, including
verified streamed scene successes and three superseded own executables. Original
inputs/expectations, negative baselines and current runners survive; replacements
and rebuilds are in `run55-cleanup.json`. No shared caches or other-lane artifacts
were removed and no bulk moved. No compression pass: `.reference` stayed below
the brief's 10 GiB trigger. Gross path savings do not attribute physical-volume
or concurrent-lane changes. Final allocated/apparent GiB: checkout **19.41/32.32**,
`.reference` **9.94/21.21**, `target` **8.03/9.48**; **29.45 GiB free**. Every ceiling
passes (`run55-disk.json`); scans include hidden paths and every apparent hard
link, with free space refreshed afterward. No artifact-heavy work follows.

### Run 54: native HTTP imageasset acquisition (2026-10-25)

Read the newest reports/messages, STATUS and full brief through Update 21.
Continued Lane F at master **6a7b48378**; other lanes and `29` are preserved.
Selected HTTP acquisition for the asset-bearing isometric boards
`stable/images`, `regression/grid_image_label_position` and
`txtar/sql-table-reserved`. This closes a foundation prerequisite; no new
whole isometric-board or pixel match is claimed.

Static source comparison found HTTP rejected before the existing image probe.
`image_http.rs`/`image_resolver.rs` now follow `lib/imageasset/resolve.go:216–294,
418–495,790–902`: fragment-free canonical requests, redacted diagnostic labels,
status/declared-length/header checks before body admission, reversed transport
encoding layers, final image probing and atomic budget charges. The explicit
`ImageHttpClient` boundary preserves injected redirect-policy failures versus
transport unavailability. `NativeImageHttpClient` implements the pinned
`lib/netpolicy` public-only dial/DNS/redirect checks, request headers, bounded
streaming and cancellable native I/O; private networks require an explicit
opt-in. Transport compilation is excluded on wasm32. The ordinary CLI retains
its existing HTTP adapter. Source mappings are in both owning SOURCE files.

| Measured tier | Before → final | Regressions |
|---|---:|---:|
| Original HTTP redirect/encoding/limit calls | **0 → 11/11** | **0** |
| Original HTTP header/fragment calls | **0 → 7/7** | **0** |
| Existing local/data-URI raster / raw SVG / SVG codecs | **17 / 8 / 7 → unchanged** | **0** |
| Original asset / LaTeX scenes | **28 / 8 → unchanged** | **0** |
| Ordinary scenebuild outcomes | **1008 → 1008** | **0** |
| Rebuilt CLI e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |

All original selected Go assertions execute. The hook records the actual
client's request/final raw HTTP response, restores its body and captures full
resource bytes/data URIs/accounting/errors. Native socket tests independently
cover redirects/headers, sniffing, public-policy rejection and cancellation of
a stalled body. The original remote-origin scene fixture additionally passes
through a real native loopback acquisition and compares its full expected scene
before/after server shutdown (with a fresh transport locator). **181 native
and four harness tests pass**; the final scene-only rerun has 38 passes.
Scoped rustfmt/Ruff/diff checks pass (pre-existing nonexecutable shebang EXE001
is ignored). No FMA, trace construction, comparator allowance or normalization
changed. Other CLI/export/raster/language/isometric tiers are inherited.

Evidence: `.reference/codex-runs/run54-{http,headers}-before.json`,
`run54-*-current{,-delta}.json`, `run54-e2e-{before,final}.json`,
`run54-e2e-delta.txt`, tests/build logs and `run54-validation.json`.
The 1008-outcome replay verifies original Go hashes and explicitly records the
three original empty/no-output inputs separately, never as matches. Post-cleanup
checks verify 939 board inputs, 1008 scene expectations, nine fonts, all 50
resource calls and 11 compressed executable hashes. Trace sources are restored
and the temporary hook detached. Latest aliases and executable retirements are
recorded by `run54-latest-bindings.json` and `run54-cleanup.json`.

Open: rooted file policy, shared caches, complete concurrent/context/transport
ownership and malformed URL/codec contracts. The header test's direct canceled
parse call/default transport total-header-cap assertion remain explicitly
uncaptured natively. General API parity is not inferred from these finite calls.
The isometric lane can now use the native HTTP resolver in its asset adapter.

The first HTTP capture missed a differently named resolver; its failed inventory
survives, and the fresh capture contains every original call. An initial added
scene test hung because its structured URL host was unchanged; it was stopped,
corrected and given a bounded listener. All subsequent tests pass. The first
scene replay's empty-input inventory failure is also retained explicitly.

Capture guards remain **1 GiB total / 128 MiB file / 20 GiB free**.
Cleanup retired **2,296 files / 307.93 MiB allocated path footprint** (mostly
verified streamed scene duplicates and four superseded own executables), plus
2,926 empty directories. `.reference` crossed the brief's 10 GiB compression
trigger during concurrent work. Applesauce **0.5.30**, verified **LZFSE**, saved
**229.51 MiB** on 11 idle single-link current runners/tests; bytes, modes,
owners and original xattrs verify, clone backups are removed and compressed
executables run. Exact targets and decompression rollback commands are in
`run54-final-compression.json`. Gross path savings are separate from physical
APFS/concurrent-volume changes. No shared caches, foreign artifacts or bulk
outside this project were moved/deleted. No commits.

Final disk: checkout **19.60/32.42 GiB allocated/apparent**, `.reference` **9.89/21.13**, `target` **8.28/9.67**; **30.36 GiB free**. Every ceiling and reserve passes (`run54-disk.json`).

### Run 53: native local/data-URI SVG acquisition (2026-10-25)

Continued the newest reports/messages and the full brief through Update 21.
The initial 20 GiB volume reserve failed, so no parity work ran until fresh
whole-checkout allocated/apparent measurements and free space passed every
gate (`run53-disk-start.json`). A concurrent volume-space recovery supplied
the headroom; this lane does not claim that recovery as cleanup savings.
Stayed in the foundation/export support scope; other lanes and `29` survive.

Selected the SVG acquisition prerequisite for `stable/images`,
`regression/grid_image_label_position` and `txtar/sql-table-reserved`.
Static comparison found `image_resolver.rs` rejected SVG before the existing
scene importer could run. The native resolver now preserves raw UTF-8/ISO-8859-1
SVG bytes, recognizes source MIME aliases/local extensions, rejects unsupported
DTDs with entities disabled, and expands gzip/zlib/raw-DEFLATE/Brotli under
caller SVG/encoded/decompressed budgets. XML depth and byte-derived work are
bounded independently of importer semantics. Kind, zero pixel dimensions,
fetched/final decoded accounting, canonical memoization and atomic cumulative
reservation follow master `lib/imageasset/probe.go:39–227,418–590` and
`resolve.go:399–413,507–576`. Sources/notices are mapped in the owning crates.

The existing exporter XML/envelope/name/codec implementation moved into
`d2-rs-math::image_svg`; exporter callers retain their legacy entry point.
The original 342 codec outcomes remain an unchanged regression fixture.
No geometry/FMA operation, trace argument construction, comparison allowance
or golden normalization changed.

| Measured tier | Before → final | Regressions |
|---|---:|---:|
| Original raw/ISO/DTD Resolve calls | **0 → 8/8** | **0** |
| Original compressed SVG Resolve calls | **0 → 7/7** | **0** |
| Existing local/data-URI raster calls | **17 → 17/17** | **0** |
| Original asset / LaTeX scenes | **28 / 8 → unchanged** | **0** |
| Ordinary scenebuild outcomes | **1008 → 1008** | **0** |

The four original SVG test functions run with Go assertions intact. Complete
resource bytes, canonical data URIs, kind/MIME, accounting and errors compare
exactly; all 15 first measured failures now match. The compression function's
direct internal cancellation call is explicitly uncaptured on the native side;
its Go assertion alone is not a native match. Native fixtures retain every
captured request/result. These finite calls do not establish full imageasset
API parity. HTTP acquisition/integration remains next, and **no new whole
isometric-board or pixel match is claimed**.

Evidence: `.reference/codex-runs/run53-{svg,codecs}-{before,final,delta}.json`,
raster asset / asset / LaTeX reports and deltas, and the 1008-row
`run53-scene-qualified.json`. The bounded scene replay reproduced the original
Go results and retired only verified successful duplicate outputs, preserving
original inputs/expectations and errors. The session-start e2e guard remains
**933 SVG / 11 expected errors**, zero regressions. The rebuilt final CLI
also preserves **67/70** shared-clock/release complete contracts and all 51
qualified goldens. Focused raster inventories pass **33/33** under Dagre,
ELK and TALA, run sequentially. cli-diff preserves every prior **115** match;
it observes **116/119** because the ordinary-clock stdout PDF happened to
share the same timestamp. That incidental row is not promoted to the stable
master baseline or counted as a new fix. Ordinary wall-clock CLI, full
317/313/313 raster, language and isometric tiers are inherited.

**390 native tests** pass (143 math, 32 scene/resolver/integration, 215
exporter), including the ten complete original SVG scenes/import-budget
outcomes through native acquisition. **53 harness tests** pass. Ruff, scoped
leaf-module rustfmt and diff checks pass; pre-existing root `lib.rs` format
differences are left untouched. An initial complete exporter test build found
two historical fixtures referencing the private helper removed during the
move. Restoring its test-only entry point makes all 215 tests pass; the
failure log survives. No production change followed the final CLI/raster runs.

Latest foundation, strict CLI and e2e report bindings are updated in
`run53-latest-bindings.json`. Original hashes verify the 1008 retained scene
outputs, nine fonts, 934 language requests, 930 bound language goldens, all
asset/LaTeX/resource capture inputs and expectations, and unchanged legacy
codec fixture. Four compressed frozen runner hashes and seven compressed
test executable hashes verify. The original imageasset trace files are
restored exactly and the temporary hook is detached.

Cleanup retired **6,438 files / 738.84 MiB allocated path footprint**:
three superseded foundation runners, the obsolete run-52 exporter test
executable, idle Go actions/completed codegen, a completed progress duplicate,
and 2,016 verified streamed scene outputs. Historical executable bindings
have explicit replacement/rebuild records; original inputs, expectations,
negative baselines and current warm runners survive. **2,549 empty
directories** were removed. Applesauce **0.5.30**, verified **LZFSE**, saves
**289.65 MiB** across 11 retained files; hashes, modes, ownership and original
xattrs match, all clone backups are removed, and compressed executables run.
These gross path savings are separate from net/concurrent volume changes;
no bulk moved. Ledgers: `run53-cleanup.json`, `run53-final-compression.json`.

Final fresh disk: checkout **18.05/30.71 GiB allocated/apparent**, `.reference` **9.50/20.70**, `target` **7.12/8.47**; **32.08 GiB free**. Every ceiling and reserve passes (`run53-disk.json`).
No commits; other lanes and `29` remain untouched.

### Run 52: master PDF page backgrounds and export regression guards (2026-10-25)

Read the newest handoffs and full brief through Update 21. Followed Update
20 first: qualify the PNG/PDF/CLI consumers of run 50's shared viewport change.
All initial master guards passed. The explicit shared-clock/release profile
then exposed four complete-PDF gaps: `internal_linked_pdf`, `stdout_pdf`,
`theme-pdf` and `no-nav-pdf`. Static comparison found master
`lib/pdf/pdf.go:119–125` paints the full page regardless of navigation.
`pdf_document::encode` now emits that background and carries FPDF's fill state
into subsequent pages in both navigation modes. Complete document bytes match;
no metadata, stream or pixel normalization was added.

| Measured tier | Before → final | Regressions |
|---|---:|---:|
| Shared-clock/release CLI strict contracts | **63 → 67 / 70** | **0** |
| Ordinary CLI contracts / goldens / applicable assertions | **58 / 51 / 67 → unchanged** | **0** |
| cli-diff | **115 → 115 / 119** | **0** |
| e2e SVG / expected errors | **933 / 11 → unchanged** | **0** |
| Full raster Dagre / ELK / TALA | **317 / 313 / 313 → unchanged** | **0** |
| Focused PNG fixtures, each engine | **33 / 33** | **0 lost matches** |

All engines ran sequentially for both raster inventories. The final focused
harness uses the manifest oracle/revision and the current built-in `tala`
name on both sides; its historical `tala-rs` spelling remains a harness alias.
An initial focused TALA invocation failed on that obsolete CLI name; its
unmodified failure report survives separately. The corrected invocation and
final harness both pass all 33 fixtures. Complete, nonempty PNG inventories
and exact decoded pixels remain the gate. Missing/corrupt/time-limited output
is still a failure.

**17 native tests + 38 harness tests pass**, plus scoped rustfmt/Ruff/diff
checks. Original PNG and image-placement requests are unchanged. Fresh pinned
Go public API calls produce separate `*-master.*` fixtures: six PNG input
inventories, four complete document witnesses and 120 aspect matrices.
Navigation-enabled PDF bytes are unchanged; master's explicit viewport product
rounding changes 17/120 matrix results. The original v0.9.0 expectations remain
historical fixtures. Reproducer: `tools/pdf-master-probe`; source mapping in
exporter `SOURCE.md`. This closes the measured export consumer check, with no
new whole-isometric-board claim. Language and unrelated isometric tiers are
inherited. Other lanes and `29` are preserved; no commits.

Evidence: `.reference/codex-runs/run52-{cli-release-clock,cli-wall}-final.json`,
their zero-regression deltas, e2e/raster before/final/delta reports,
`run52-validation.json`, source fixture manifest and native/harness logs.
Latest master export guards are updated. After cleanup, original hashes verify
all 1008 retained scenes, nine fonts, 934 language requests, 930 bound goldens
and the old/new exporter fixtures; all 328 e2e cases remain.

A volume free-space stop halted one capture before execution. Captures/builds
paused until cleanup and fresh whole-checkout measurements passed every gate.
Deletion/compression ledgers preserve inputs, expectations and negative PDF
documents. One compressed old test output was subsequently rebuilt by Cargo;
its old binding is explicitly superseded, rather than claimed to retain its
old bytes. Gross path savings and final whole-checkout disk measurements are
in `run52-cleanup.json`, `run52-compression.json`, `run52-disk.json` and
`report-run52.md`. No bulk moved outside the checkout.

Final disk: checkout **18.76/31.12 GiB allocated/apparent**,
`.reference` **10.57/21.73**, `target` **6.75/7.76**;
**15.68 GiB free**. Project size ceilings pass, but **the 20 GiB
volume-free floor fails** after a second volume drop following qualification.
No further parity captures/builds ran after that drop. Restore headroom and
prove the floor passes before resuming parity development. Actual retirement:
**7,394 files / 3536.05 MiB allocated path footprint**;
verified compression saves **1014.35 MiB** across current retained
versions. Gross path savings are separate from physical/concurrent volume
changes; no bulk moved. Raw measurements and overlays: `run52-disk.json`.

Next foundation work remains HTTP/SVG acquisition for `stable/images`,
`regression/grid_image_label_position` and `txtar/sql-table-reserved`;
ordinary no-resolver errors remain explicit. Sketch work still requires a
named failing board. Lane I owns complete isometric assembly.

### Run 51: native local/data-URI raster acquisition (2026-10-25)

Continued the newest reports and the full brief through Update 19 within the
foundation lane at master **6a7b48378**. Static comparison chose the next
asset prerequisite for `stable/images`, `regression/grid_image_label_position`
and `txtar/sql-table-reserved`: the scene handoff accepted probed resources,
but no native foundation resolver produced them. This run closes its local/
data-URI raster slice; those named HTTP/SVG boards still require acquisition
integration and are not counted as fixed. Other lanes and `29` are preserved.

Added `RasterImageResolver` at `AssetOptions.resolver_impl`, with explicit
trusted local-file I/O, finite positive caller budgets, bounded percent/base64
decoding, metadata-only raster probes, canonical source memoization, owned data
URIs, cancellation checks and atomic cumulative accounting. Failed operations
do not consume the budget or poison retries. Source mapping:
`lib/imageasset/resolve.go:72–254,368–413,507–576,687–720`,
`probe.go:230–359`, `imageasset.go:70–112,168–218`; scenebuild `SOURCE.md`
records the native mapping. No diagnostic/FMA sites or comparator allowances
were added.

| Measured tier | Before → final | Regressions |
|---|---:|---:|
| New original local/data-URI Resolve calls | **0 supported → 17/17** | **0** |
| Original scene asset / LaTeX calls | **28/8 → 28/8** | **0** |
| Ordinary scenebuild outcomes | **1008 → 1008** | **0** |
| e2e SVG / expected errors | **933/11 → 933/11** | **0** |

The new scoreboard captures four original test functions with all Go assertions
intact and explicitly inventories **63 uncovered functions** across six test
files. Full resource bytes, metadata, data URI, accounting and complete errors
match. Native integration also builds the complete original Go image/icon scene
directly from the data URI. **27 native tests and 14 harness checks pass**;
scoped rustfmt/Ruff/diff checks pass. A first scene integration assertion used
raw JSON numeric representations; the final test uses its existing typed scene
comparison. The complete inventory gate rejected Finder `.DS_Store`; the
disposable metadata was retired and the unchanged gate rerun. Every original
scene/font hash and captured request/result verifies; trace tests are restored.

Evidence: `.reference/codex-runs/run51-imageasset-{baseline,qualified-final,delta}.json`,
asset/LaTeX qualified reports and deltas, `run51-scene-qualified.json`, e2e
before/final/delta, validation and test logs. New source calls are permanent in
`imageasset-local-data-master.jsonl`; final runners are frozen and latest
foundation reports updated. Capture guards: **1 GiB total / 128 MiB file /
20 GiB free**. Both e2e guards use the unchanged frozen run-50 CLI, which does
not call the new resolver; unrelated language/export/isometric tiers are
inherited. Changes remain uncommitted.

Next: bounded HTTP/SVG acquisition for the named image boards, then rooted
file policy, shared caches and context/concurrent-loader contracts. APNG and
animated WebP still use unsupported static admission; GIF animation metadata
is supported. The ordinary no-resolver corpus keeps its 84 image errors and
27 no-import-options errors. No whole isometric board or new pixel match is
claimed. Cleanup/compression and final disk figures are recorded in
`run51-cleanup.json`, `run51-compression.json`, `run51-disk.json` and
`report-run51.md`.

Cleanup retired **2057 files / 359.26 MiB allocated path footprint**, plus
**2016 verified streamed outputs / 225.73 MiB footprint** and 2545 empty
directories. Four obsolete foundation executables have source/rebuild retirement
overlays. Verified Applesauce LZFSE on four retained executables saved
**42.63 MiB**, preserving hashes, modes, ownership and original xattrs; all
temporary backups are removed. Original inputs/expected outputs and negative
baselines verify. These gross path figures do not attribute physical free-space
changes during concurrent lane writes; no bulk moved.

Final allocated/apparent GiB: checkout **21.03/32.36**, `.reference`
**9.68/20.83**, `target` **9.93/9.93**; **36.58 GiB free**. Every ceiling
passes (`run51-disk.json`, raw du/df). Target/reference allocation and free space
were refreshed after the full scans. No capture or build followed this audit.

### Run 50: bounded SVG imports and LaTeX scene labels (2026-10-25)

Continued the latest reports, full brief through Update 19, and the final iso3
handoff. Chose Update 18's shared import prerequisite: ten original SVG asset
calls and 24 failing rich-label boards, including `stable/latex` and
`regression/grid_with_latex`, all three engines. Foundation implementation only;
Lane I/L source and `29` remain untouched. No commits by this run.

Connected scenebuild to the existing native math SVG importer. The bridge
preserves complete owned primitives/paints/clips/children, cumulative import
limits, max(parsed, emitted) work charges, sorted atomic embedded-raster merge,
content deduplication and clipped intrinsic viewports. LaTeX validates input,
limits and currentColor before caching by formula/color, renders native MathJax,
shares image import budgets, hashes exact SVG/color bytes and places labels at
intrinsic imported dimensions. Source: `assets.go:143–173,257–299,325–425` and
`latex.go:30–173`; mapping in scenebuild `SOURCE.md`.

Five LaTeX calls exposed an older importer arithmetic path. Master's
`d2scene/aspect.go:28–54` explicitly rounds products and computes translation
directly. Replaced the older FMA/matrix-composition helper. `viewport.go` itself
is unchanged since v0.9.0. The existing 2,280-row viewport fixture now obtains
its transform values/bits from pinned Go's public AspectRatioMatrix; old
expectations survive separately. No comparator tolerance or exclusion changed.

| Measured tier | Before → final | Regressions |
|---|---:|---:|
| Original image/icon Build calls, complete scenes/errors | **18 → 28/28** | **0** |
| Original LaTeX Build calls, complete scenes/errors | **2 → 8/8** | **0** |
| Rich-label nonempty boards | **168 → 192/192** | **0** |
| Rich-label exact calls, retained SVG + premultiplied pixels | **402 → 438/438** | **0** |
| Ordinary scenebuild inventory | **1008 → 1008** | **0** |
| e2e SVG / expected errors | **933 / 11 → 933 / 11** | **0** |

The new LaTeX scoreboard inventories all six original test functions and eight
Build calls, keeping every Go assertion (including its source raster checks).
All 192 original nonempty rich boards were freshly replayed through the rebuilt
native runner, including every prior match. Their retained source input hashes
and independently reproduced Go SVG golden bindings verify. The 678 empty rich
inventories and 138 Go errors are inherited from iso3, not a new Go capture.
All 1008 ordinary Go/native scene outcomes independently reproduce the retained
source inventory; the 84 no-resolver and 27 no-import-options errors stay errors.
Native mesh and whole isometric SVG remain **0/870**. Structured cells bypass
the rich painter; full isometric, general SVG and all public API parity remain
open. Other golden/CLI/export/language tiers are inherited.

**164 native tests** (141 math, 17 scene library, three fallback and three
asset/LaTeX example tests) and **11 harness checks** pass. Scoped rustfmt, Ruff
and diff checks pass. Original source tests and assets.go are restored exactly.
Capture/replay guards: **1 GiB total / 128 MiB file / 20 GiB free**; fresh scene
peak **9.15 MiB**, largest **4.58 MiB**. No new capture or build follows the final
disk audit. Compression is deferred per brief Update 3 while `.reference` stays
below 10 GiB allocated.

Evidence: `.reference/codex-runs/run50-{assets,latex}-{before,qualified,delta}.json`,
`run50-rich-{final,delta}.json`, `run50-scenebuild-{qualified,delta}.json`,
e2e before/final/delta, native/harness logs, `run50-corpus-check.json`,
`run50-validation.json` and `report-run50.md`. Latest master scene/assets/LaTeX/
rich reports updated by atomic replacement; prior report bindings are retained.
Rebuild: `cargo build --profile diagnostic -p d2-rs-scenebuild --example
asset-probe --example scenebuild-probe`; replay via the asset/LaTeX scoreboards
with `--reuse`. The rich replay script verifies all 192 retained original inputs.

Next foundation item: native imageasset fetch/probe integration for the supplied
resolver boundary, starting with named image boards `stable/images`,
`regression/grid_image_label_position` and `txtar/sql-table-reserved`. HTTP/file/
data-URI loading and resolver budgets/cancellation remain outside this handoff.
Lane I can now use its existing positive SVG limits for LaTeX; it still owns
mesh/aperture/coverage integration and whole-renderer qualification.

Cleanup retired **four obsolete files, 196.76 MiB allocated / 196.75 MiB
apparent**, including the completed math test executable and three superseded
snapshots. Additionally retired **2208 verified streamed files, 238.88 MiB
allocated path footprint**, and **2807 empty directories**. Deletion/rebuild
records: `run50-cleanup.json`. All 1008 original scene hashes, nine font inputs,
28 asset calls, eight LaTeX calls and 192 rich inputs verify after cleanup;
original negative results remain. No bulk moved. Shared Go actions remain
while other lanes run. Gross path retirement is separate from physical-volume
savings, APFS sharing and concurrent growth.

Final allocated/apparent GiB: checkout **21.06/32.29**, `.reference`
**9.86/20.88**, `target` **9.83/9.80**; **23.99 GiB free**. Every ceiling
passes (`run50-disk.json`, raw du/df; target refreshed after full scans).
Reference allocation/free space were refreshed again before completion;
whole/apparent values are the preceding full scans during concurrent writes.

### Run 49: raster image/icon scene resources and unavailable placeholders (2026-10-25)

Foundation lane, continuing the latest reports and the complete brief through
Update 19. Chose Update 18's image/icon prerequisite ahead of LaTeX. Named
resource-error boards include `stable/images/{dagre,elk,tala}`,
`regression/grid_image_label_position/{dagre,elk,tala}` and
`txtar/sql-table-reserved/{dagre,elk,tala}`. These need explicit asset
resolution before native isometric admission; no whole-board gain is claimed.
Isometric/language edits and `29` remain untouched. No commits by this run.

Added `AssetOptions.resolver_impl: Option<Arc<dyn ImageResolver>>`, immutable
already-probed resources and typed unavailable/fatal errors. Build caches by
source before I/O, deduplicates by SHA-256 of kind/MIME/bytes, owns raster
byte copies, and uses Go's shared 64x64 vector placeholder only for unavailable
sources. Layout validation still precedes resolver I/O. Marker-only options
preserve their previous unsupported behavior. No new floating-point sites.
Source: `d2scenebuild/assets.go:103–179,182–256,301–323`; mapping and API
limitations are in the crate SOURCE.md.

| Measured tier | Before → final | Regressions |
|---|---:|---:|
| Original asset Build calls, full scenes/errors | **8/28 → 18/28** | **0** |
| Ordinary scenebuild boards | **1008 → 1008** | **0** |
| e2e SVG / expected errors | **933 / 11 → 933 / 11** | **0** |

The new asset scoreboard inventories all 14 functions in upstream
`assets_test.go`: twelve original Build functions produce 28 calls; the two
internal merge/cancellation functions remain explicitly uncovered. All
selected original assertions run, including resolver counts and Go raster
checks. A trace-only observer captures actual resolver bytes/metadata/errors;
the Rust adapter replays that boundary and checks resolver calls. Complete
scenes, every raster byte, font-byte SHA-256 and exact errors compare with the
existing typed comparator. **Ten SVG-import calls remain non-passing**.
The 18 closed contracts cover shape/ordinary/structured/code/connection
images, owned JPEG orientation metadata, unavailable deduplication, fatal
limits/data errors, opacity admission and invalid layouts before I/O.

**22 native tests and 10 harness checks pass**; focused rustfmt, Ruff and
diff checks pass. All 28 original baseline outputs are independently
reproduced byte-exactly with the final runner's `--legacy` mode. The new
permanent fixture preserves all 28 Go calls, including unresolved SVG cases.
Original trace files are restored byte-exactly. Internal-package file mode
keeps every selected asset assertion while avoiding a second production
build without the test-only observation hook; an initial ordinary-package
attempt's compile failure remains recorded. No comparator allowance or
timeout was changed.

Fresh complete Go/native scene inventories reproduce all 1008 bound outcomes.
Captures used **1 GiB total / 128 MiB per file / 20 GiB free** guards; source
asset peak **35.27 MiB**, final scene peak **9.15 MiB**. Verified passing
scene replays were retired as they finished. Every retained scene expected
output, font input and new asset input/result hash verifies. E2e uses the
unchanged frozen CLI, which does not call this foundation API; other tiers
are inherited.

Evidence: `.reference/codex-runs/run49-assets/{before,qualified}.json`,
`run49-assets-delta.json`, `run49-scenebuild-{qualified,delta}.json`,
`run49-e2e-{before,final}.json`, `run49-e2e-delta.txt`, native/harness logs,
`run49-corpus-check.json`, and `report-run49.md`. Latest master reports:
`scenebuild-assets.json` and `scenebuild.json`.

Next: implement bounded SVG import (the ten measured asset failures, shared
with the 27 LaTeX boards), then native fetch/probe integration for supplied
resolvers. The ordinary no-resolver corpus's 84 asset errors and 27 LaTeX
errors remain errors. This handoff does not expose native HTTP/file/data-URI
loading, resolver budgets/cancellation, or qualify native pixels from these
new documents. Existing CLI asset loading is unchanged. Lane I can supply
already-probed raster resources through the new interface; its integration
and full-board qualification remain separate.

The subsequent iso3 handoff measures **24 failing rich-label boards / 36
calls** requiring LaTeX scene import, including
`regression/grid_with_latex/{dagre,elk,tala}` and
`stable/latex/{dagre,elk,tala}`. Use that concrete failing inventory
(`run49-next-targets.json`) to drive the shared SVG/LaTeX importer next.

Cleanup retired **76 idle obsolete files, 83.35 MiB allocated / 83.11 MiB
apparent**, plus **4032 verified streamed outputs, 451.47 MiB allocated path
footprint**. `run49-cleanup.json` and `run49-executable-retirements.json`
identify replacements and rebuilds; original negatives and all Go inputs/
expected results remain. No bulk moved; gross path retirement is separate
from physical-volume savings and concurrent growth. Target briefly exceeded
10 GiB after the initial build; scoped deletion and a fresh whole-checkout
audit preceded further work. Final allocated/apparent GiB: checkout
**20.91/32.13**, `.reference` **9.66/20.78**, `target` **9.80/9.77**;
**29.03 GiB free**. Every ceiling passes (`run49-disk.json`, raw du/df).
No artifact-heavy work followed this audit. Compression is deferred under
brief Update 3 while `.reference` remains below 10 GiB allocated.

### Run 48: shared bundled font fallback for scene builds (2026-10-25)

Foundation lane, after reading the complete brief through Update 17 and the
run47/iso2/language handoffs. Chose Lane I's measured
`unicode/emojis/{dagre,elk,tala}` blocker: scenebuild rejected every configured
font resolver. Added `FontFallbackOptions.resolver_impl`, a native immutable
resolver boundary and `BundledFallbackResolver` with bounded authenticated
Noto loading, cumulative request/byte limits and optional downstream
delegation. Scene builds now admit content-addressed font resources, select
fallbacks per text reference, and reuse the bundled face across style buckets.
Existing placeholder-only calls retain their behavior. No isometric or
language source was edited, and no new floating-point sites were added.

Source: `d2scenebuild/font_fallback.go:246–410`,
`d2fonts/bundled_fallback.go:18–302`, `fallback.go:588–622`,
`noto_color_emoji_native.go`, and
`internal/fontfallback/fontfallback.go:157–189`. The crate SOURCE.md records
the mapping and platform/ownership limits.

| Measured tier | Before → after | Regressions |
|---|---:|---:|
| Original bundled-fallback Build calls, complete scenes/errors | **0/4 → 4/4** | **0** |
| Ordinary scenebuild boards | **1008/1008 → 1008/1008** | **0** |
| e2e SVG / expected errors | **933 / 11 → 933 / 11** | **0** |

The new scoreboard retains all assertions of three original Go tests: emoji
build/render admission and its one-byte-short font budget, variation-selector
base coverage, and regular/italic bucket reuse. It compares full Build scene
JSON, every glyph and font-byte SHA-256, or exact errors. Constructor limits
come from the actual Go resolver instances. **17 library + 3 integration
tests and 5 harness tests pass**; native reuse requires one resolver call per
build. Focused rustfmt, Ruff and diff checks pass. Native raster pixels for
these new documents are not separately qualified by this Build tier.

The initial broad replay rejected the old capture's 898 extras (897 model
replays and one `.DS_Store`); that negative remains explicit. Fresh Go and
Rust inventories reproduce every retained expected scene/error, using
1 GiB total / 128 MiB file / 20 GiB free guards and retiring verified passing
outputs as they finish. No exclusions or comparator allowance were added.
Evidence: `.reference/codex-runs/run48-final/{before,after}.json`,
`run48-fallback-delta.json`, `run48-scenebuild-final.json`,
`run48-e2e-{before,final,delta}.json`, test logs and
`run48-source-inventory-negative.json`.

Lane I can now configure `resolver_impl: Some(Arc::new(
BundledFallbackResolver::new(None, limits)?))` with its existing font budgets.
Its fallback painter/coverage integration and qualification remain Lane I's
work; **no whole
isometric board gain is claimed here**. The scene model owns font Vecs per
Document; Go's cross-Document slice sharing, mutable public d2fonts resolver
API, host font discovery and cancellation are not ported by this boundary.
Other CLI/export/language and raster results are inherited. Next foundation
queue remains named band/session/cancellation failures, CFF/dfont and an
upstream COLRv0 painting witness. Cleanup, final disk audit and handoff are
in `run48-cleanup.json`, `run48-disk.json` and `report-run48.md`.

Retired **26.82 MiB allocated / 26.78 MiB apparent** of obsolete
executables/replays, plus **451.47 MiB allocated path footprint** of verified
streaming captures. Unmeasured interrupted-attempt retirements are recorded
separately and excluded from savings. No retained corpus inputs or canonical Go expected outputs
were retired. Final allocated/apparent GiB: checkout **20.70/31.94**,
`.reference` **9.50/20.59**, `target` **9.76/9.73**;
**35.40 GiB free**. Every ceiling passes; path retirement is separate from
physical-volume free-space change and concurrent growth. No bulk moved;
compression deferred per brief Update 3. No commits.

### Run 47: shared first-frame normalization and default PNG API (2026-10-25)

Foundation lane, following the entire brief through Update 17 and the newest
run-46/iso1/lang1 handoffs. Exposed `d2_rs_raster::normalize_first_frame_png`
and `encode_png_default` for Lane I's `vector_surface_svg.go:455–496` request.
No isometric or language source was edited. The initial shared adapter copied
Lane I's qualified static-PNG-only boundary; all **23 newly captured first-frame
requests were unsupported**. Eight original `internal/rasterimage` tests now
match complete PNG bytes: JPEG EXIF orientations/malformed-EXIF fallback,
GIF first-frame canvas placement, APNG poster exclusion/default-frame control,
and static/animated WebP (including unknown padded chunks).

Ported the container inspection/first-frame materialization and Go default PNG
model/filter/zlib/IDAT behavior. JPEG reuses the native core decoder; lossy WebP
uses VP8 planes and exact integer YCbCr conversion. A partial GIF's palette
ColorModel survives its logical-canvas wrapper, selecting PNG16 rather than
PNG8 (`firstframe.go:95–114`, Go `image/png/writer.go:627–663`). A focused
source/over APNG witness initially matched **1/2**, then **2/2** after preserving
`draw.Draw`'s NRGBA copy versus integer premultiply/unpremultiply fallback.
Seven existing static-PNG model records and two NRGBA stride/origin encodings
also match Go. No floating-point sites were added.

| Measured tier | before → after | regressions |
|---|---:|---:|
| first-frame normalization, complete PNG bytes | 0/23 → **23/23** | 0 |
| supplemental APNG source/over bytes | 1/2 → **2/2** | 0 |
| existing raster text/color API | 10/10 → **10/10** | 0 |
| e2e SVG / expected errors | 933 / 11 → **933 / 11** | 0 |

**34 foundation tests + 11 new gate tests + 52 existing raster-gate tests pass.**
Source captures are bounded by 1 GiB total / 128 MiB file / 20 GiB free, with
original Go assertions retained. The new gate compares all encoded bytes and
requires complete input/outcome inventories; missing, extra, ambiguous,
changed and failed-process results cannot pass. Evidence:
`.reference/codex-runs/run47-images-{before,sealed}.json`, their delta,
`run47-alpha-{before,final}.json`, `run47-api-qualified.json`, e2e before/final/delta,
and native/harness test logs; source mapping in raster `SOURCE.md`.

This adds a shared foundation API, not end-to-end isometric board matches.
Lane I can now replace its local adapter; its integration/mesh queue remains
separate. The renderer's old private decoder is unchanged. Arbitrary malformed
assets, native cancellation, RenderSession and D2's BestSpeed PNG/GIF APIs
remain unqualified/unexposed; `encode_png_default` specifically follows
`image/png.Encode`. Other CLI/export/language/scene tiers were not freshly
measured: these additive helpers are outside those callers' paths. Disk and
retirement evidence is in `run47-cleanup.json`, `run47-disk.json` and the
run-47 report. Retired **26 files, 6.54 MiB allocated /
6.45 MiB apparent**: four idle superseded executables and
22 byte-verified replaced replay PNGs. The GIF negative and all Go inputs/
expected outputs remain. An active project Go process prevents action-cache
retirement. No bulk moved; compression deferred per Update 3. Final allocated/
apparent GiB: checkout **20.46/31.75**, `.reference` **9.34/20.42**,
`target` **9.75/9.66**; **33.91 GiB free**. Every ceiling passes.
Scoped staging failed twice with `index.lock: Operation not permitted`;
changes remain unstaged/uncommitted. `run47-git.json` lists the eleven own
paths; Cargo.lock's raster dependency hunk is separate from Lane I's hunk.

### Run 46: raw text and color-font raster API parity (2026-10-25)

Foundation lane, following the full brief through Updates 15–16; isometric
and language lane edits are preserved. Added `scripts/raster-api-scoreboard.py`
around six original pinned upstream raster tests. Its ten actual Render
requests initially had **1 match / 9 failures**: the raster port still
rejected raw text and color glyphs despite the completed fontface crate.
The failing witnesses are the text-anchor/underline/missing-glyph cases,
`TestGradientPaintsTextFillAndStroke`, Noto palette/SoftLight rendering, and
the repeated-color-glyph structural-limit cases. Go's original assertions,
including the two emoji PNG hashes, all passed before native comparisons.

Connected `PreparedFont` to the shared fontface reader/shaper and removed
the interim raster-only sfnt reader. Bundled fonts share immutable outlines
and create shaping faces lazily; raw text uses the render-local shaping
workspace and its aggregate input/coverage/run/glyph budgets
(`d2raster/text.go:141–184,461–597`). Ported ordered user/palette/foreground
text parts, COLRv0 layers, and authenticated COLRv1 glyphs, transforms,
clips, gradients, SrcIn masks and isolated SoftLight groups
(`text.go:220–458`, `colrv1.go:27–366`). Plans are cached, while each glyph
occurrence is still charged. The one-below/exact node-limit pair now matches
Go, including the complete failing-glyph error. New gradient contractions
were checked against Go 1.27 ARM64 instructions; source mapping is in
`crates/d2-rs-raster/SOURCE.md`.

| Measured tier | before → after | regressions |
|---|---:|---:|
| upstream raster text/color API outcomes | 1/10 → **10/10** (8 RGBA images, 2 errors) | 0 |
| complete scene-raster images / render-limit error | 896 / 1 → **896 / 1** | 0 |
| e2e SVG / expected errors | 933 / 11 → **933 / 11** | 0 |

The scene-raster replay freshly renders every retained Go Document; its
111 scene-build failures still have no raster input and remain non-passing.
The e2e guard reruns the unchanged CLI snapshot; this crate is not in the
CLI dependency closure. Other lanes' changes and their current scoreboards
are separate work, not qualified by these numbers. No whole-e2e-board gain
is claimed. **30 foundation tests, 16 new API-gate tests and all 36 existing
raster-gate tests pass**, as do focused Ruff/rustfmt/diff checks.

The API harness retains each test's exact FrameOptions and compares all four
decoded channels, including RGB under zero alpha. Its strict PNG parser
extends the existing opaque-only gate's validation; missing cases, inputs,
results, corrupt/truncated PNGs, ambiguous outcomes and extra files fail.
The existing raster comparator is unchanged. Captures stayed under the
1 GiB total / 128 MiB file / 20 GiB free guards. Evidence:
`.reference/codex-runs/run46-api-{baseline,qualified,delta}.json`,
`run46-raster-{final,delta}.json`, `run46-e2e-{before,final,delta}.json`,
test logs, `run46-colrv1-instructions.txt`, and `report-run46.md`.

This closes the measured raw-text/COLRv1 gap, not all raster APIs. COLRv0
painting is source-translated but lacks a current captured source witness;
RenderSession, native context cancellation and this crate's PNG/GIF encoder
APIs remain unexposed. Fontface's CFF/dfont limitations still apply. Next
foundation work should start from named failing upstream API tests; the
isometric mesh/full-SVG queue belongs to Lane I. Final cleanup and storage
measurements are recorded in `run46-cleanup.json` / `run46-disk.json` and the
run-46 handoff report. Retired 935 files (**83.33 MiB allocated / 81.52 MiB
apparent**): byte-verified superseded native replays, two idle raster
executables, the large assembly dump (compact function/instructions retained),
and the replaced interim sfnt module. The old native replay also contained a
generated `.DS_Store`; its explicit negative inventory row and minimal binary
witness are preserved in `run46-raster-retirement-audit.json`. The current
897-file raster output inventory is complete and has no extras. All 897 Go
scene inputs/results, their nine font files, and 43 API-capture files remain
verified. No bulk moved. Scoped Git staging succeeded, but commit failed on
`index.lock: Operation not permitted`, including the required retry; only the
22 foundation/tooling paths are staged. Shared STATUS and concurrent lockfile
changes are unstaged; a later SOURCE.md notice edit is also unstaged. Final
allocated/apparent GiB: checkout **19.94/31.39**, `.reference` **8.89/20.24**,
`target` **9.56/9.53**; **41.96 GiB free**. Every ceiling passes; compression
remains deferred per Update 3. These are sequential snapshots during concurrent
work; gross cleanup savings are separate from net checkout change.

### Run 45: qualify foundations and fix shared matrix fusion (2026-10-25)

Read the full handoff through Update 14; work is against master `6a7b48378`,
not the superseded v0.9.0 queue. Qualified the inherited fontface and scene
builder ports, then fixed `d2-rs-scene::Matrix::{mul,point,vector}` from
`d2scene/geometry.go:214–234` and the pinned Go 1.27 ARM64 instructions.
Go rounds the second product and fuses the first; translation is added
separately. The shared Rust implementation had fused the second product.
Removed the scene builder's duplicate `matrix.rs` workaround and its trait.

Witness: `stable/cross_arrowhead` rotates the center of a 17×17 marker.
The new ARM64 regression test failed before the fix and passes afterward,
checking ten exact output components against independent Go Mul/Point/Vector
call sites. The source and compiler-instruction witness are retained as
`run45-matrix-go.json`, `run45-matrix-instructions.txt` and
`witness-inputs/run45-matrix.go` under `.reference/codex-runs/`. Go may reuse
rounded SSA products when several APIs are inlined together; the witness
keeps their call sites independent. No whole-board gain is claimed: the
scene builder already had the workaround.

| Regression tier | before → after | regressions |
|---|---:|---:|
| fontface records | 5068 → 5068 | 0 |
| scene-builder documents/errors | 1008 → 1008 | 0 |
| scene model round trips | 897 → 897 | 0 |
| scene raster images / render error | 896 / 1 → 896 / 1 | 0 |
| isometric BuildScene / writer-only SVG | 1008 / 870 → 1008 / 870 | 0 |
| e2e SVG / expected errors | 933 / 11 → 933 / 11 | 0 |
| language goldens / editor source+errors | 915 / 445 → 915 / 445 | 0 |

All affected tiers were freshly captured or replayed against retained Go
documents. The writer remains the explicit **surface-stubbed** tier; its
138 Go resource errors are non-passing. Raster has 896 pixel-exact PNGs and
one exact `d2raster: frame pixels exceed limit 67108864` result for
`real_world/tpmjs_architecture/dagre`. Another 111 boards fail scene building
before a raster Document exists; they remain non-passing in the raster
tier and are independently matched in the scene-builder tier. The initial
raster report also preserves its extra `.DS_Store` negative; that generated
capture file was inventoried and retired, not excluded from a comparator.

Restored the isometric crate's default build by explicitly gating the
unfinished mesh-builder modules with `cfg(any())`. All WIP source remains;
the unfinished retained-surface implementation is
`img/vector_surface_wip.rs`, while `vector_surface.rs` supplies the existing
writer-only identity boundary. The isometric examples and crate tests build;
full native mesh/surface/SVG rendering remains unimplemented. **30 native
foundation tests and 36 raster-gate tests pass.** No production comparator,
normalization, deadline or exclusion changed. Existing CLI/export/inline
binaries were not rebuilt: Cargo metadata confirms those dependency closures
do not include any of the six new renderer crates. Their prior master
scoreboards are inherited, not fresh measurements in this run.

One aggregate capture hit its 1 GiB guard and was stopped; no partial output
was counted. The writer rerun partitions by case family, retains all 1008
rows, and peaks at 597.34 MiB (largest mesh 29.62 MiB). Final guards use the
same 1 GiB total / 128 MiB file / 20 GiB free limits for each run. Evidence:
`run45-*-final.json`, `run45-*-delta.*`, test/guard logs and
`run45-unaffected-dependencies.json`. Git staging initially succeeded for
the foundation files, but subsequent index operations failed with
`index.lock: Operation not permitted`, including the required retries.
No commits; review the partially staged index before committing.

Next: finish raster's unexercised color-font/API paths from named source
witnesses (not another replay of the already matching 896 images), then
complete and enable the isometric mesh port, add its Go mesh-dump tier,
implement full vector-surface serialization, and score the 870 complete SVG
goldens. Audit each two-product FMA site against the compiled caller before
enabling it. The CLI `--isometric` flag/help follows the renderer.

Cleanup and final disk evidence are recorded in `run45-cleanup.json`,
`run45-disk.json` and `report-run45.md`; all retained current capture font
references and pinned board inputs are checked before handoff.
Retired 20,945 files: **3.75 GiB allocated / 4.16 GiB apparent** reclaimed
(allocated accounting uses the measured target reduction instead of counting
hard-linked compiler objects twice). Final allocated/apparent GiB: checkout
**21.00/32.24**, `.reference` **9.07/20.16**, `target` **9.47/9.44**;
**39.29 GiB free**. All ceilings pass; no bulk moved. Compression remains
deferred per Update 3 (`.reference` below 10 GiB allocated).

Concurrent isometric surface modules, hook and scoreboard edits appeared
during handoff. They are preserved and are not run-45 changes; the results
above qualify the writer-only binaries built earlier in this run, not that
subsequent surface work. Review current source and index before building or
committing it. No claim of full isometric/API parity is made.

## SVG parity: d2svg port (goal S1–S6, see PLAN)

### CLI behaviour against the pinned Go CLI (2026-10-24)

New `scripts/cli-diff-scoreboard.py` runs 80 invocations (subcommands, help,
usage and flag errors, every export format, multi-board outputs) through the
pinned Go CLI and d2-rs in fresh directories and compares exit code, stdout
and stderr exactly, normalizing only durations and the program path. Before:
4/29 of the first error-path probes matched; now **78/80**
(`.reference/cli-diff-scoreboard-20261024/latest.json`). Fixes, all from
d2cli/main.go, help.go, fmt.go, validate.go, export.go and xmain.mainFatal:
every user-facing invocation parses the pinned flag set first, so pflag's
unknown-flag/needs-argument/invalid-argument usage errors and font loading
come from one boundary; usage errors carry mainFatal's `Run with --help`
trailer; `layout` counts positionals and prints the plugins' Short/LongHelp
with Defaults() flag tables and layoutNotFound; `fmt`/`validate` require a
path, resolve ms.AbsPath and report Go's `open <path>` errors (validate parses
without a source path); `themes` ignores extra arguments; theme lookup,
stdout-format and too-many-arguments errors match; negative `--pad` and
`--animate-interval` pass through as in Go (SVG output identical); bool flags
accept `--sketch=false`; `--dark-theme` warns and is dropped for non-SVG;
PDF/PPTX/GIF and animated multi-board exports log success; compile
diagnostics use the absolute source path. The CLI harness now models
mainFatal exactly: only `err:` lines form the error, and the usage trailer is
not part of it. CLI scoreboard 57 → **58/70** (`layout-extra-args`), e2e SVG
933 unchanged, zero regressions everywhere.

Two known misses: `--stdout-format=pdf` (fpdf's wall-clock CreationDate) and
`fmt --check` with unformatted files, where upstream `fmt-check-unformatted`
asserts the in-process error chain (`failed to fmt: exiting with code 1: …`)
while the Go process prints only the ExitError message (and a timestamped
WARN line on stdout); d2-rs keeps the contract the upstream test checks.

### Raster parity complete for every renderable board (2026-10-24; latest)

PNG export now emulates d2cli's `RenderBands` destinations (256-row bands,
`effectRenderBounds` layers, `setBandReferenceBounds` references; see
`crates/d2-rs-export/SOURCE.md`). Raster scoreboard: **Dagre 317/317,
ELK 313/313, TALA 313/313** — every e2e board Go itself renders (5 Dagre
boards fail in Go). From 934: +5 Dagre (`regression/opacity-on-label`,
`stable/animated`, `txtar/c4-person-label`, `txtar/md-label`,
`txtar/theme-overrides`), +4 ELK (same three plus `txtar/icon-style`), zero
regressions. GIF and paged frames keep the full-frame path (Go uses
`Render` there). Other gates unchanged: e2e SVG 933 (delta 0/0), CLI
58/70 contract-match (case-level identical), cli-diff 78/80, flag matrix
302/304 (`stable/chaos2` under theme 300, 7 px, twice), golden tiers sketch
38, dark 12, appendix 6, exporter 19. A band record pass skips mask coverage,
which halves mask-heavy PNG time (`stable/arrowhead_scaling` 31 s → 17 s).

### Rendering under non-default flags (2026-10-24)

New `scripts/flag-matrix-scoreboard.py`: 8 upstream e2e boards x 19 flag sets
(themes 0/1/4/8/101/104/200/300/303, sketch, pad, center, scale, dark theme,
ELK, TALA, sketch+terminal) x SVG/PNG against pinned Go. 279 → **285/304**
(`.reference/flag-matrix-scoreboard-20261024/latest.json`). Fixed: a mono theme
now replaces sketch's HandDrawn primary family (SVG font faces), arrowhead
labels use the primary (mono-theme) italic face, and raster sketch tables use
d2scenebuild's header padding, round caps and mono text. The 19 left are the
banding sub-pixel nits (`txtar/c4-person-label` under every flag, 2×
`stable/chaos2` terminal). Raster 934/943 and e2e SVG 933 unchanged, zero
regressions.

### Raster parity across the e2e corpus (2026-10-24; latest)

New `scripts/raster-scoreboard.py` renders every applicable e2e board to PNG
with pinned Go and d2-rs and compares decoded pixels exactly
(`compare_png_outputs`). Baseline: **866/943** pixel-identical, Dagre 293/317,
ELK 284/313, TALA 289/313 (Go itself fails 5 Dagre boards). The 77 mismatches
come from ~35 boards, 19 of them on all three engines: text/markdown/CJK,
fill patterns and themes (origami, 3d-sides), border radius on tables, animated
boards, a few 1–114 pixel anti-aliasing cases, and one canvas-size difference
(`txtar/width-smaller-than-label` 604 vs 602 px). Baselines:
`.reference/raster-scoreboard-20261024/<engine>-latest.json`.
Fixes since: outside-label canvas bounds truncate like Diagram.BoundingBox
and rectangle radii clamp like clampBorderRadius (875/943); class/sql_table
outer boxes, headers and final rules follow buildStructuredShape's radius:
878/943; ZWJ emoji sequences paint through the native COLR path instead of
falling back to `.notdef` (881/943); COLRv1 emoji contribute their clip box
to Markdown paint bounds instead of discarding them, which had erased other
text in the same Markdown label (884/943); fill-pattern overlays follow
interleavePattern (multiply class everywhere, front copy only for `multiple`,
outer box + header only for class/sql_table) (890/943); animated connections
split at their midpoint like splitAnimatedConnectionPath (901/943); Markdown
headings wrap with the shared atomized wrapInlineTokens (904/943); outside
Markdown labels sit outside the shape (905/943); font fallback resolves one
scene-wide bucket per text font like resolveFontFallbacks (911/943); routes
honour NoCornerRadius themes (917/943); static WebP icons decode through
x/image/webp's YCbCr/NRGBA model, animated shapes composite as isolated layers,
and non-solid fill/stroke/text paint follows drawRasterizedPaint (byte-domain
composite, subpath pixel-bounds origin, unfused x-only axis coordinate, f64
gradient vector, label Ink box for text gradients) (926/943); unset label
positions stay at the inner box's top-left, and ELK places container→descendant
edges like elk-go (port net flow, breadth-first split order, label segment,
route offsets): **931/943** (Dagre 312, ELK 309, TALA 310), zero regressions;
render, export and ELK tests pass; e2e SVG 933/933 unchanged.

Remaining 9 (after the TALA seed-2 fix below): 5 Dagre boards Go itself
fails; sub-pixel nits of 1–114 pixels where glyph or stroke coverage lands on the
other side of a quantization edge (`regression/opacity-on-label` Dagre+ELK,
`stable/animated`, `txtar/theme-overrides`, `txtar/c4-person-label` Dagre+ELK,
`txtar/md-label` Dagre+ELK, `txtar/icon-style` ELK).

Root cause of the remaining sub-pixel raster nits (traced on
`txtar/theme-overrides` with a dense-path hook added to the instrumented Go
rasterizer): d2cli renders PNG through `d2raster.RenderBands` with 256-row
bands (`raster_png_output.go` pngBandHeight). In each band an effect node
(opacity, mask, isolation) gets a layer tightened by `effectRenderBounds` /
`strokeRegionBounds`, and every rasterizer clips to its destination: edges
left of the layer accumulate into `difference[0]`, rows outside it are not
scanned, and `flattenCube` drops cubics wholly above or below it (so round
cap/join disks of off-band dashes vanish). d2-rs rasterizes full-frame with
a window anchored at the reference origin, so the same edges sum in a
different f32 order. Example: the dashed `user -> network.portal.UI` route at
(2299,455) has identical partial-area bits but winding `b5300000` vs Go's
`b5700000` (layer min x 887 vs 0), coverage 98 vs 97. A faithful fix is a
rasterizer window equal to Go's destination per 256-row band (edges offset by
reference − window, as `setRasterizerReference` does), with effect windows
from `effectRenderBounds`; d2-rs's existing banded mode is not qualified for
this (forcing it gives ELK 254/313, TALA 255/313). The masked-group `−2`
reference shift in `draw_group` approximates the layer clamp and should go
with that change.

TALA seed-2 gap (found by the raster sweep, which renders with the CLI's
default seeds while every scoreboard pins `--tala-seeds=1`): fixed
2026-10-24. `stable/investigate`, `stable/large_arch` and
`real_world/tpmjs_architecture` now equal pinned Go at seeds 1, 2, 3 and the
default. Three causes (details in `crates/d2-tala-rs/SOURCE.md`): AlignAxes
sequence abductions tested no route obstruction (an empty recorded slice stood
in for Go's live Containers walk, and that walk let a sequence vessel block its
own edges); Graph.direct dropped edges reconnected to a cluster vessel; and the
scope fit centred cluster members on a pre-herd-flip vessel size. Seed-2/3
sweep of every e2e case: +3, zero regressions; e2e SVG 933/933 unchanged.
Raster: TALA 310 -> **313/313**, so the raster total is **934/943** (Dagre 312,
ELK 309, TALA 313), zero regressions.

### Claude after the second Codex window (2026-10-23)

Codex runs 18–44 were reviewed and committed one by one. Independently
re-scored from a fresh build afterwards (zero regressions everywhere): e2e
**933/933** applicable SVGs, CLI **57/70** cases / **46/61** goldens (clock-
and release-pinned profiles 62 and 66), language **915/445**, sketch **38/39**,
exporter **19/19**.

New tiers: d2svg `dark_theme` **12/12** and `appendix` **6/6**
(`scripts/golden-tier-scoreboard.py --tier dark|appendix`). Fixes found by them:
`RenderOptions::hand_drawn_font` (HandDrawn export font without sketch config)
and Go's `Graph.IDA` self-link rule in validateBoardLinks, which keeps a nested
board's link to itself (`TestAppendix/internal-links`). Language and render
tests pass.

TALA graphjson fixtures (`d2layouts/d2talalayout/internal/testdata/{layout,ovg}`)
against the rebuilt direct Go oracle: layout **44/44** (was 42/44) and
post-layout snapshots **78/78** (was 76/78); reports
`docs/graphjson-{layout,postlayout}-boundary.json`. The oracle build scripts
used the retired `.reference/go-mod` cache layout and could not rebuild offline;
they now use the canonical `.reference/go-mod/pkg/mod`.

Open: `d2renderers/d2raster/testdata` (2 PNG goldens) renders hand-built target
diagrams through d2scenebuild/d2raster; d2-rs has no target-JSON raster entry
point (its raster path starts from the graph), so these stay unscored.

Inline-expectation tiers (`scripts/inline-scoreboard.py`): d2format TestPrint
**35/35** and escape **28/28**; d2lsp completion **40/40**, up from 26/40 after replacing the
lexical completion heuristics with a port of completion.go's
getKeywordContext (AST-based context, `stored_data`, LabelPositionsArray for
`near`, arrowhead maps). Remaining inline candidates: d2svgimport (50),
d2ir pattern (49), lib/imageasset (34), d2ast (28). d2svgimport and d2raster test internal
d2scene/d2raster structures, which d2-rs has no counterpart for (its raster
path renders the SVG target natively).

### Run 44 continuation (2026-10-23)

Read the newest reports/messages, STATUS and the full brief through Update 12.
Baseline `a3cab8f`; no commits; inherited untracked `29` preserved. Startup
scoreboard confirms **933 SVG matches / 11 expected errors**. Existing upstream
artifact defects are closed under their explicit profiles; selected the queued
older PDF convenience serializers after establishing named failing source
fixtures: default/none/speed/best/split-IDAT, mixed, opaque-default with a link.

- Routed `render_pdf`, `package_pdf_pages` and
  `package_pdf_pages_with_link_regions` through the qualified FPDF document
  writer. Static comparison found an unconditional PDF 1.4 header, missing
  font resources, separate Info/date encoding and incorrect centered-image
  link X coordinates in the older writer. Removed over 300 lines of duplicate
  serialization and allocation. Source: D2 `lib/pdf/pdf.go:30–40,72–118,168–190`;
  FPDF v0.12.0 `fpdf.go:209,4271–4349,4979–5059`.
- **13 public API/retained-source document pairs improve 0 → 13 exact**,
  including both empty-link and ordinary packaging, six-page deduplication,
  and a centered image's hyperlink rectangle. Original fixtures are unchanged.
  The regression uses the documented shared UTC clock in an isolated process.
  Ordinary convenience exports now share FPDF's local-time date syntax.
- Fresh pinned Go reproduces all five single-page compression encodings and
  the linked document exactly (**6/7 documents**). Its mixed document permutes
  equal-width image objects: `fpdf.go:4742–4773` ranges a map and stable-sorts
  only by width, even with `SetCatalogSort(true)`. Complete image-object bytes
  form the same multiset; page/font/Info objects are exact. The differing Go
  replay remains `witness-inputs/run44-mixed-go-map-order.pdf`. Matching the
  original mixed fixture does **not** establish deterministic Go replay.

| scoreboard tier | before → run 44 | regressions |
|---|---:|---:|
| CLI release/shared-clock artifacts | 66/70 → unchanged | **0** |
| CLI source/shared-clock / ordinary artifacts | 62/70 / 57/70 → unchanged | **0** |
| CLI assertions / historical goldens | 66/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All scoreboards and all three CLI profiles freshly captured with the rebuilt
native binary; exporter assertions **2/2**. **217 exporter + 216 CLI unit tests
pass**. Existing format tests now assert source PDF syntax and follow actual
image/mask references instead of the retired object numbers. Focused Rust
formatting, `git diff --check`, current binary hashes, seven latest reports
and **403 golden capture files** verify. Capture guards: **1 GiB total /
128 MiB per file / 20 GiB free**. No comparator, normalization, exclusion,
raster harness or release-version changes. Supplementary API gain only; no
upstream whole-artifact gain is claimed.

Evidence: `run44-*-final.json`, seven deltas, `run44-pdf-api-delta.json`,
`run44-source-reproduction.json`, `run44-validation.json`, before/final witness
and unit-test logs. Reproducer: `witness-inputs/run44-pdf-convenience.go`;
temporary Go helper removed. Initial CLI scoring lacked Pillow; completed
captures were rescored with `uv --with pillow`, without recapturing or changing
any verdict rules.

Handover: keep all three CLI profiles distinct. The excluded plugin, upstream
skip and two unsupported injected-writer rows remain visible. Broader
native/WASM/platform APIs still require named failing source witnesses; the
convenience PDF writer duplication is closed. Preserve the equal-width Go
map-order witness; do not normalize it or infer arbitrary PDF reproducibility.
Full D2/API parity remains incomplete.

Cleanup removed **4,315 files**, recovering **388.19 MiB allocated / 373.56 MiB
apparent** (idle Go actions and verified duplicate source captures), plus the
temporary trace helper. Original negative baselines, current captures,
executables and corpus inputs/outputs remain; no bulk moved outside the
checkout. Ledger: `run44-cleanup.json`; compression deferred per brief Update 3.
Final allocated/apparent GiB: checkout **18.84/31.02**,
`.reference` **7.31/18.43**, `target`
**9.54/10.44**; **78.58 GiB free**. All ceilings
pass (`run44-disk.json`, raw du/df). Gross cleanup savings are separate from
capture/build growth.

### Run 43 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the full brief through Update 12.
Baseline `5abfb99`; no commits; inherited `29` preserved. Startup e2e confirms
**933 SVG matches / 11 expected errors**. Selected the largest remaining
artifact cluster: `how_to_solve_problems_pptx`, `pptx-theme-overrides`,
`stdout_pptx`, `no-nav-pptx`.

- Qualified all four **complete PPTX archives byte for byte** against pinned
  Go built with its standard v0.9.0 release linker stamp. The version flows
  from `d2cli/main.go:644` through `lib/pptx/pptx.go:285–288` into AppVersion;
  `ci/release/_build.sh:57` supplies the release stamp. No native defect was
  found or native implementation changed.
- Added opt-in `--pptx-release` to the CLI scoreboard. It runs those original
  Go tests in a separate release-stamped binary, preserving ordinary source
  assertions/captures and their explicit version mismatches. Both sets of
  assertions must pass. Complete release/native bytes compare exactly.
  Historical golden qualification still uses only the ordinary source run.
  The manifest binds the profile; reuse rejects profile changes, and missing
  source artifacts or release events cannot pass. No metadata rewriting.
- Combined with run 42's shared clock, the explicit release/shared-clock
  profile reaches **66/70 artifacts** (source/shared-clock: **62/70**).
  This is qualification under an additional build profile, not four newly
  repaired native outputs. The separate alias is `latest-release-clock.json`.

| scoreboard tier | before → run 43 | regressions |
|---|---:|---:|
| CLI release/shared-clock artifacts | source profile 62/70 → release profile **66/70** | **0** |
| CLI source/shared-clock / ordinary-clock artifacts | 62/70 / 57/70 → unchanged | **0** |
| CLI source assertions / historical goldens | 66/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

Fresh complete CLI source/native captures plus four release-Go test cases;
fresh e2e and language runs. Source-clock/wall and sketch/exporter captures
rescored with their original native hashes; **403 retained golden files**
verify, exporter assertions **2/2**. **17 harness tests**, focused Ruff and
diff checks pass. No Rust build/test rerun: native code/binaries are unchanged.
Capture guards remain **1 GiB total / 128 MiB per file / 20 GiB free**.
Evidence: `run43-*-final.json`, `run43-e2e-before.json`, seven delta files,
`run43-validation.json`, `run43-cli-gate-tests.log`.

Handover: all **66 currently representable, in-scope CLI cases** now match
artifacts under release/shared-clock controls. Remaining rows stay visible:
`layout-extra-args` (external plugins, excluded), `linked-path` (upstream skip),
`stdout_pdf_partial_write_error` and `stdout_pptx_close_error` (injected writers
unsupported by the process adapter). Keep all three profiles distinct. Broader
native/WASM/platform APIs and older PDF convenience serializers remain open;
establish a named failing Go witness before further implementation work.
Do not repeat completed encoder, metadata, subtree, IDAT or clock work.
Full D2/API parity remains incomplete.

Cleanup removed **4,167 files**, recovering **368.64 MiB allocated / 354.50 MiB
apparent**, primarily idle Go actions and a verified duplicate rescore.
Current captures, source-profile negatives, native executables and prior corpus
bindings remain. No bulk moved; compression deferred per brief Update 3.
Ledger: `run43-cleanup.json`. Final allocated/apparent GiB: checkout
**18.76/30.94**, `.reference` **7.29/18.40**, `target` **9.48/10.38**;
**80.11 GiB free**. All ceilings pass (`run43-disk.json`, raw du/df outputs).
Gross cleanup savings are separate from capture growth.

### Run 42 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the full brief through Update 12.
Baseline `b69fa46`; no commits; inherited `29` preserved. Startup e2e confirms
**933 SVG matches**. Selected the five remaining PDF artifact mismatches:
`internal_linked_pdf`, `stdout_pdf`, `theme-pdf`, `renamed-board`, `no-nav-pdf`.

- Added documented, opt-in `SOURCE_DATE_EPOCH` support to both native PDF
  writers. Decimal Unix seconds in `0..=253402300799` select UTC creation and
  modification dates; invalid values return export errors. Ordinary clocks
  retain their previous behavior. Three public packaging APIs pass two valid
  instants and three invalid values in isolated subprocesses, including a
  non-UTC host timezone. Calendar/range boundaries also pass.
- The CLI scoreboard's explicit `--pdf-epoch` profile configures the same
  instant on both sides. Trace-only instrumentation uses FPDF's documented
  `SetCreationDate`, `SetModificationDate`, `SetCatalogSort` controls
  (`fpdf.go:4178–4220`, reached from `lib/pdf/pdf.go:Init`). The native writer
  already emits that stable catalog order. Controls are recorded in manifests;
  reuse rejects changed/missing clock profiles. The comparator is unchanged.
- **All five complete upstream PDFs now match every byte** under the shared
  epoch `946782245`: clock-controlled artifacts **57/70 → 62/70**, zero
  regressions. Original page-count/positive-dimension assertions remain intact.
  The ordinary-clock profile is separately recaptured and remains **57/70**;
  this is reproducible-clock qualification, not ordinary wall-clock identity.
  Historical PDF goldens still require independent Go reproduction.

| scoreboard tier | before → run 42 | regressions |
|---|---:|---:|
| CLI shared-clock artifacts | 57/70 → **62/70** | **0** |
| CLI ordinary-clock artifacts / assertions / goldens | 57/70 / 66/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five scoreboards fresh; exporter assertions **2/2**. **216 exporter + 216
CLI unit tests**, **15 harness tests** and **10 Go clock subtests** pass. New
module/focused Rust formatting, focused Ruff (`E9,F`) and diff checks pass;
inherited formatting is preserved. No raster harness, normalization, exclusion,
deadline or release-version changes. Four PPTX rows retain the explicit pinned
source/release AppVersion mismatch. External plugins remain excluded.

Evidence: `run42-*-final.json`, six delta files, `run42-validation.json`,
`run42-{export-test-final,cli-test,cli-gate-tests,go-clock-tests}.log`.
Both CLI profiles and the original pre-change clock capture remain. Six latest
aliases, native/source hashes and **403 golden capture files** verify.
Streaming CLI/golden captures retain **1 GiB total / 128 MiB per file /
20 GiB free** guards. The ordinary CLI remains `latest.json`; the controlled
profile is separately published as `latest-clock.json`.

Next: keep the two CLI profiles distinct. The four PPTX version rows, two
unsupported injected-writer rows and excluded plugin rows remain visible.
Broader native/WASM/platform API gates and older convenience PDF serialization
remain open; name a failing source witness before investigating them. Do not
repeat the completed encoder/metadata/subtree/IDAT repairs. Full D2/API parity
remains incomplete.

Cleanup removed **4,324 files**, recovering **381.59 MiB allocated / 366.93 MiB
apparent**: idle Go actions and disposable duplicate/intermediate checks.
Current deliverables, original negatives, qualified captures and prior corpus
bindings remain; no bulk moved. Ledger: `run42-cleanup.json`. Compression
deferred per brief Update 3. Final disk audit: allocated/apparent GiB: checkout **18.75/30.93**, `.reference` **7.28/18.39**, `target` **9.48/10.38**; **79.35 GiB free**. All ceilings pass. Audit: `run42-disk.json` and raw du/df; gross cleanup savings are separate from build/capture growth.

### Run 41 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the full brief through Update 12.
Baseline `9e4eb13`; no commits; inherited `29` preserved. Startup e2e confirms
**933 SVG matches**, zero regressions. Selected the queued run 37 opaque-PNG
PDF failure; the remaining upstream rows are clock/order, source/release
metadata, excluded plugins and unsupported injected writers.

- `decode_pdf_png` retains validated, non-interlaced RGB8 IDAT payloads.
  Both PDF writers consume those original bytes, including content-derived
  image IDs/deduplication. Source: FPDF v0.12.0 `png.go:120–124,247`, reached
  from D2 `lib/pdf/pdf.go:86`; `def.go:generateImageID`. Native generated
  rasters and existing RGBA splitting retain their established encoder.
- Run 37's original **6992-byte negative source PDF now matches all bytes**.
  Its original source/native witnesses remain unchanged. New complete Go
  single-page documents improve **1/5 → 5/5** (default, none, speed, best,
  split IDAT). A further six-page mixed-compression document matches all bytes,
  including image deduplication. Initial mixed fixture reused one board ID and
  hit FPDF registration caching; it was corrected to distinct IDs and its
  initial verdict is not used for improvement counts.
- Three public PNG packaging APIs preserve the exact source image stream for
  all five encodings (**15 contracts**); five damaged-PNG controls reject.
  Complete source fixtures cover **10 PDFs** total, including the existing
  three BestSpeed/nav/Unicode documents. Fixed clock/catalog controls exist
  only in the Go witness/private tests; product clocks and comparators remain
  unchanged, per Update 12.

| scoreboard tier | before → run 41 | regressions |
|---|---:|---:|
| CLI assertions / artifacts / goldens | 66/70 / 57/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five fresh; exporter assertions **2/2**. **214 exporter + 216 CLI unit
tests pass**. The new PDF gain is supplementary; no upstream whole-artifact
gain is claimed. Five PDF strict rows and four PPTX source/release AppVersion
rows stay visibly non-passing. Focused rustfmt and diff checks pass. No raster
harness, normalization, exclusion, deadline or release-version changes.

Evidence: `run41-*-final.json`, five deltas, `run41-validation.json`,
`run41-witness-{before,final}.log`, `run41-original-witness.log`, qualified
test/build logs. Reproducer: `witness-inputs/run41-pdf-png.go`; fixtures:
`pdf-png-idat-fpdf.json`, `pdf-opaque-default-fpdf.pdf` in exporter tests.
Streaming captures use **1 GiB total / 128 MiB per file / 20 GiB free** guards.
Latest aliases, current binary hashes and 403 golden capture files verify.

Next: older convenience PDF writers need a named failing source document
before further changes; broader native/WASM/platform API gates remain open.
Caller-supplied grayscale/indexed/16-bit/interlaced PNG and arbitrary RGBA
filter preservation are not qualified by this RGB8 fix. Do not repeat the
completed PNG encoders/metadata, subtree packaging or this IDAT repair.
Full D2/API parity remains incomplete; external plugins remain excluded.

Cleanup retired **4,517 files**, recovering **710.25 MiB allocated / 695.07 MiB apparent**:
idle Go actions, completed LTO objects, an obsolete oracle exporter test
executable, duplicate witness captures and the temporary trace helper. Current
deliverables, original negatives and qualified captures remain; no bulk moved.
Ledger: `run41-cleanup.json`. Compression deferred per brief Update 3.
Final allocated/apparent GiB: checkout **18.72/30.90**, `.reference` **7.25/18.37**, `target` **9.48/10.38**; **79.35 GiB free**. All ceilings pass.
Audit: `run41-disk.json`, raw du/df files; gross deletion savings are distinct
from net build/capture growth.

### Run 40 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the full brief through Update 12.
Baseline `9e4eb13`; no commits; inherited `29` preserved. Startup e2e confirms
**933 SVG matches**. The remaining live upstream failures are PDF clock/order,
PPTX source/release version metadata, excluded plugins and injected writers.
Selected the queued standalone PNG gap: 12 upstream PNG output files already
matched pixels and IDAT, but lacked the complete CLI eXIf chunk.

- Standalone PNG output now inserts D2's big-endian TIFF Make/Model metadata
  after IHDR, with exact lengths, offsets, NUL termination and CRC. Source:
  `lib/png/png.go:22–39,66–87,128–136,199–229` and
  `d2cli/raster_png_encoder.go:305–318`. Internal PDF/PPTX image encoders retain
  their metadata-free bytes. Native release version remains `v0.9.0`.
- **Complete PNG files 0/12 → 12/12 byte-identical** to the pinned release-stamped
  Go CLI: hello_world_png, hello_world_png_pad, png-with-local-icons,
  hello_world_png_sketch and first/second outputs of geometry, markdown,
  local-icon and unlabelled-connection-link. The existing complete inventory/
  pixel gate also passes every file. No PNG metadata stripping or normalization.
- Seven additional process contracts cover file, stdin/stdout, root/named
  subtree traversal and `--omit-version`: all **10 PNG files** match complete
  release Go bytes. Retained fixture plus `scripts/check-png-metadata.py`
  reproduce them. Fourteen complete Go PNG encoder fixtures cover RGB/RGBA,
  inline/offset string boundaries, Unicode bytes and source/release models.

| scoreboard tier | before → run 40 | regressions |
|---|---:|---:|
| CLI assertions / artifacts / goldens | 66/70 / 57/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five fresh; exporter assertions **2/2**. **212 exporter + 216 CLI unit tests
pass**, plus seven retained PNG process cases. PNG upstream totals are unchanged
because that tier already compares decoded pixels; the complete-byte gain is
reported separately. Source-only Go retains its `v0.8.1-HEAD` metadata. No
comparator, exclusion, deadline, release-version or raster-harness changes.
Focused rustfmt, Python Ruff and `git diff --check` pass.

Evidence: `run40-*-final.json`, five deltas, `run40-png-cli.json`,
`run40-png-boundaries.json`, `run40-validation.json` and build/test logs.
Generators: `witness-inputs/run40-png-{exif.go,cli.py,boundaries.py}`.
Both PNG witnesses pass explicit streaming **64 MiB total / 128 MiB per file /
20 GiB free** guards. Scoreboards retain their **1 GiB / 128 MiB** guards.
Latest aliases, executable/capture hashes and pinned source inventories verify.

Next: arbitrary opaque input PNG IDAT preservation in PDF package APIs (retain
run 37's two negative PDFs), then older convenience PDF writers and remaining
API/platform gates. PDF clock policy stays per Update 12: original page/
dimension contracts plus explicit part evidence; five strict PDF rows and four
PPTX source/release version rows remain visibly non-passing. Do not repeat
completed PNG encoders/metadata or subtree fixes. Full D2/API parity is incomplete;
external plugins remain excluded.

Cleanup retired **4,545 files**: idle Go actions, completed LTO
objects, superseded witness copies and the temporary trace-checkout Go helper. Recovered **695.86 MiB allocated / 680.59 MiB apparent**; ledger
`run40-cleanup.json` retains hashes and rebuild/replacement records. No bulk
moved; current executables, original negative witnesses and qualified captures
remain. Compression deferred per brief Update 3 (`.reference` below 10 GiB
allocated). Final allocated/apparent GiB: checkout **18.79/30.97**, `.reference` **7.23/18.35**, `target` **9.57/10.46**; **79.35 GiB free**. All ceilings pass.
Audit: `run40-disk.json` and raw final du/df files; gross removals are separate
from net build/capture growth.

### Run 39 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the full brief through **Update 12**.
Baseline `2dc4216`; no commits; inherited `29` preserved. Session-start e2e
confirms **933 SVG matches**. Chose the remaining named-subtree PDF/PPTX queue
item: live upstream failures are PDF clock/order, PPTX source/release version,
excluded plugin flags and unsupported injected writers. Per Update 12, retain
PDF's upstream page/dimension contract plus explicit page evidence; no clock
injection or timestamp normalization was added.

- `board.*` PDF/PPTX exports now retain the selected board's descendants,
  resolve shorthand/quoted names through the compiler and package only that
  subtree. A non-wildcard selection keeps only the selected board.
- Paged traversal rebases IDs to `root`, removes outside ancestors and uses
  the selected root's label/navigation policy. Absent labels use board names;
  explicit empty labels remain empty. Folder-only parents produce no page.
  Selecting a folder without children returns Go's no-renderable-board error.
- Source: `d2cli/main.go:368–385,539–549,610–667`,
  `d2cli/paged.go:66–76,408–553,656–683`, `d2exporter/export.go:25–28`.
  CLI SOURCE.md updated. Targets: nested/shorthand, quoted mixed collections,
  folder/leaf, named/root stdout, stdin-file/stdin-stdout, default label,
  single-board/root-only selections; supplementary documents **5/24 → 24/24**.
  All **23 PDF pages** match dimensions, drawing streams, used RGB/alpha images
  (decoded and compressed) and annotations. Full raw PPTX files **4/12 → 12/12**
  against the release-stamped pinned Go CLI. Two additional folder-selection
  error contracts pass, including nonzero exits and empty output inventories.

| scoreboard tier | before → run 39 | regressions |
|---|---:|---:|
| CLI assertions / strict artifacts / goldens | 66/70 / 57/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five fresh; exporter assertions **2/2**. **216 CLI unit tests pass**;
`scripts/check-paged-subtrees.py` passes all **26** retained Go contracts.
Supplementary gains are distinct from upstream totals. Five PDF strict rows
still differ by timestamps/order; four PPTX rows retain the explicit source
`v0.8.1-HEAD` versus release `v0.9.0` AppVersion difference. No comparator,
normalization, exclusion, deadline, release-version or raster-harness changes.
Focused rustfmt, Python Ruff and `git diff --check` pass.

Evidence: `run39-*-final.json`, five deltas, `run39-paged-{before,qualified}.json`,
`run39-validation.json`, qualified build/test logs. Generator:
`witness-inputs/run39-paged-subtree.py`; fixture:
`crates/d2-rs-cli/tests/fixtures/paged-subtree-source-v090.json`.
Latest aliases and native capture hashes verify; golden manifests' retained
files verify after cleanup. Pinned pristine source/oracle verified. Streaming
1 GiB total / 128 MiB per file / 20 GiB free guards pass; witnesses use 64 MiB.

Next: standalone PNG eXIf, arbitrary opaque PNG IDAT preservation, older PDF
convenience writers and remaining API/platform gates. Named-subtree packaging
is qualified only for these finite cases, not arbitrary boards/links or full
D2/API compatibility. External plugins remain excluded.

Cleanup retired **5,509 files**: superseded CLI/exporter executables,
completed thin-LTO objects, idle Go actions and this run's intermediate captures.
Recovered **884.38 MiB allocated / 936.51 MiB apparent**; ledger `run39-cleanup.json` records paths,
hashes and replacements. Initial negative witnesses and qualified captures
remain; no bulk moved. One macOS `.DS_Store` reappeared and is recorded.
Compression deferred per brief Update 3 (`.reference` below 10 GiB allocated).
Final allocated/apparent GiB: checkout **18.85/31.04**, `.reference` **7.21/18.34**, `target` **9.65/10.55**; **80.87 GiB free**.
All ceilings pass, including hidden paths/.git and every apparent hard-linked
path. Audit: `run39-disk.json`, raw final du/df files; gross cleanup savings
are separate from net build/capture growth.

### Run 38 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the full brief through Update 11.
Session-start scoreboard confirms **933 SVG matches**, zero regressions.
Baseline `dce5c47`; no commits; inherited `29` preserved. Selected the remaining
named-wildcard packaging/stdio gap after run 33's SVG/PNG fix; PDF clock/order
and PPTX source/release AppVersion failures remain visibly non-passing.

- Fixed GIF `board.*` selection: resolve shorthand/quoted names through the
  compiler, retain only the selected subtree, and start frame collection at
  that board. Previously only its first board reached GIF export, silently
  dropping descendants. Folder-only parents remain omitted from frames.
- Composed multiboard GIFs now support file/stdin/stdout boundaries. Stdout
  returns the complete artifact, rather than rejecting it or writing `-`.
- Source: pinned `d2cli/main.go:368–385,539–549,582–608` and
  `d2cli/raster.go:580–629,929–978`. CLI SOURCE.md updated. Nine complete
  source-CLI witnesses improve **1/9 → 9/9**, covering nested/shorthand,
  quoted mixed collections, folders/leaves and process I/O. All **19 decoded
  frames**, canvases, delays and loop counts match Go using the unchanged
  complete GIF gate. This is supplementary coverage, not an upstream score gain
  or a claim of raw GIF byte identity.

| scoreboard tier | before → run 38 | regressions |
|---|---:|---:|
| CLI assertions / strict artifacts / goldens | 66/70 / 57/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five fresh; exporter assertions **2/2**. **217 tests pass**: 216 CLI unit
tests and one process regression covering all nine complete Go fixtures.
The process regression also checks output inventories and the absence of a
literal `-` file. Repeated it against the final formatted binary. No comparator,
normalization, exclusion, deadline or release-version changes; raster harness
unchanged. Focused rustfmt and `git diff --check` pass.

Evidence: `run38-*-final.json`, five delta files, `run38-gif-{before,after}.json`,
`run38-validation.json`, build/test logs; generator
`witness-inputs/run38-gif-subtree.py`, fixture
`crates/d2-rs-cli/tests/fixtures/gif-subtree-source-v090.json`, process regression
`tests/gif_subtree.rs`. Pinned pristine source and Go 1.27.0 oracle hash verified.
Latest aliases updated. CLI capture **5,535,536 bytes**, largest **723,311**;
streaming 1 GiB total / 128 MiB per file / 20 GiB free guards pass. GIF witness
outputs also stay below their 32 MiB budget. The scored executable snapshot is
retained; a final rustfmt-only line wrap changed the rebuilt binary hash, with
its process regression rerun. Golden-tier runner hashes match their captures.

Next: named-subtree PDF/PPTX selection remains open (not changed here), as do
standalone PNG eXIf, arbitrary opaque input PNG preservation and older PDF
convenience writers. Five PDF strict rows remain blocked by date/map ordering;
four PPTX rows retain the explicit source/release AppVersion difference. Full
D2/API parity remains incomplete; external plugins remain excluded.

Cleanup retired **4,286 idle Go action-cache files**, recovering **378.79 MiB
allocated / 364.25 MiB apparent**. Pinned dependencies, current executables,
all corpus bindings and before/after GIF witnesses remain. Cargo locks and
scoped cache-open check pass. Ledger: `run38-cleanup.json`; no bulk moved.
Compression deferred per brief Update 3 (`.reference` below 10 GiB allocated).
Final allocated/apparent GiB: checkout **18.89/31.15**, `.reference` **7.14/18.27**, `target` **9.75/10.72**; **82.02 GiB free**.
All ceilings pass, including hidden paths/.git and every apparent hard-linked
path. Audit: `run38-disk.json`, raw final du/df files. Savings are gross cache
removals, separate from net build/capture growth.

### Run 37 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the entire brief through Update 11;
session-start e2e confirms **933 SVG matches**, zero regressions. Targeted the
five failing PDFs: `internal_linked_pdf`, `renamed-board`, `no-nav-pdf`,
`theme-pdf`, `stdout_pdf`. Baseline `daebca2`; no commits; inherited `29` preserved.

- Replaced the CLI/navigation PDF serializer with FPDF's page/resource object
  structure, inline annotations, shared resources, image deduplication,
  compressed drawing streams, literal UTF-16 cell text, font/image SHA-1 IDs,
  UTF-16 Producer, catalog and xref layout. Unix dates now use local wall time
  and source syntax; non-Unix hosts currently fall back to UTC.
- Source: D2 `lib/pdf/pdf.go:AddPDFPage`; FPDF v0.12.0 `fpdf.go:803–910,
  2667–2840,3405–3428,4271–4384,4742–4908,4979–5108,5222–5285` and
  `def.go:233–255,856–860`. Mapping and root/exporter notices updated.
- Actual eight PDF pages: decoded **and compressed** drawing streams
  **0/8 → 8/8**; image resource IDs **0/8 → 8/8**. All RGB/alpha pixels and
  compressed image streams remain exact. Complete object-graph comparison
  records only ten changed values: creation/modification times across five
  PDFs, with a bijection of every indirect reference. This is supplementary
  analysis, not a normalized match verdict. Three raw PDFs differ only in
  date bytes; two also have dictionary/object ordering differences.
- Three complete deterministic Go documents match **every raw byte**, covering
  navigation, Unicode, unequal heights, links, opaque images and duplicate
  images. Only the witness uses FPDF's existing catalog/date controls; product
  clocks and oracle captures remain unchanged.

| scoreboard tier | before → run 37 | regressions |
|---|---:|---:|
| CLI assertions / strict artifacts / goldens | 66/70 / 57/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five fresh; exporter assertions **2/2**. **No whole-artifact gain claimed.**
No comparator, deadline, normalization, exclusion or release version changed.
**427 tests pass** (211 exporter, 216 CLI). Existing tests now decode compressed
page commands and check the source's numeric/string encoding. Initial font-ID
registered/embedded-name confusion and Producer/catalog bytes were corrected
from complete source fixtures. The opaque fixture initially used default PNG
compression; qualification uses D2's BestSpeed input. The arbitrary-input
opaque PNG stream-preservation gap remains explicit, with both negative PDFs
and reproduction retained (`run37-retained-witnesses.json`).

Evidence: `run37-*-final.json`, all five deltas, `run37-pdf-analysis.json`,
`run37-validation.json`, logs; `witness-inputs/run37-{pdf-documents.go,
extract-documents.py,analyze-pdfs.py}`; checked-in `pdf-documents-fpdf.json`.
Go **1.27.0** enforced. Latest aliases updated and checked; retained golden
capture hashes verify. CLI capture **5,535,525 bytes**, largest **723,311**;
all captures pass unchanged streaming budgets. New modules pass rustfmt;
diff checks pass. Unrelated formatting preserved; raster harness unchanged.

Next: keep the five PDF rows visibly non-passing for clock/map ordering, and
four PPTX rows for source/release AppVersion. Native PDF page/resource byte
serialization is now qualified on these cases; avoid repeating it. Remaining
actionable supplementary gaps include standalone PNG eXIf, broader subtree
packaging/stdio, arbitrary opaque input PNG byte preservation and older PDF
convenience writers. Full D2/API parity remains incomplete; plugins excluded.

Builds briefly exceeded `target/`'s ceiling; parity work paused, then resumed
only after cleanup and a fresh whole-checkout allocated/apparent/free-space
audit passed (`run37-disk-gate-*`). Cleanup retired **6,463 files**,
recovering **1118.04 MiB allocated / 1043.68 MiB apparent**: idle Go actions, obsolete LTO objects/CLI/oracle test
executables and disposable successful/debug copies. Current warm runners,
all upstream captures and unresolved witnesses remain. Ledger:
`run37-cleanup.json`; no bulk moved outside the project. Compression deferred
per brief Update 3 (`.reference` below 10 GiB allocated).

Final allocated/apparent GiB: checkout **18.77/31.03**, `.reference` **7.13/18.26**, `target` **9.65/10.62**; **66.38 GiB free**.
All ceilings pass, including hidden paths/.git and every apparent hard-linked
path. Audit: `run37-disk.json` and raw final du/df files. Cleanup savings are
gross measured removals, separate from builds' net storage change.

### Run 36 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the full brief through Update 11.
Targeted the largest remaining cluster: PDF strict artifacts
`internal_linked_pdf`, `renamed-board`, `no-nav-pdf`, `theme-pdf`, `stdout_pdf`.
No commits; inherited `29` preserved. The supervisor committed run 35 as
`e90cc11` during handoff.

- Native PDF font embedding now follows FPDF's seven-object UTF-8 structure,
  descriptor metrics, CID width grouping, fixed-size CID map and BestSpeed
  font/map compression. Both registered fonts are embedded even with
  navigation disabled, matching `lib/pdf/pdf.go:34–35`.
- FPDF retains an older cmap length formula than D2's SVG font helper.
  A PDF-specific entry point selects that formula; the default SVG path
  retains D2's correction. Source mapping: FPDF v0.12.0 `fpdf.go:4503–4711`,
  `utf8fontfile.go:335–447,608`, `xcompr.go:25–44`.
- On the five CLI PDFs, embedded font inventories improve **2/10 → 10/10**;
  exact compressed font programs **0/10 → 10/10**, exact compressed CID maps
  **0/10 → 10/10**, and width tables **0/10 → 10/10**. All eight page image
  inventories, dimensions and decoded/compressed RGB and alpha streams match.
  Six supplementary complete seven-object fixtures also match every raw byte.

| scoreboard tier | before → run 36 | regressions |
|---|---:|---:|
| CLI assertions / strict artifacts / goldens | 66/70 / 57/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five scoreboards are fresh; exporter assertions **2/2**. No complete PDF
artifact gain is claimed. Outer serialization, page streams/resource identifiers
and timestamps still differ. No comparator, normalization, exclusions, deadlines
or release version changed. Full D2/API parity remains incomplete.

**465 tests pass**: 210 exporter, 216 CLI, 39 text library. New/modified font
modules pass rustfmt; diff checks pass. Existing unrelated lib.rs formatting
is preserved. Raster harness unchanged. Evidence: `run36-*-final.json`, five
deltas, `run36-font-analysis.json`, `run36-validation.json`, test/build logs.
Reproducers: `witness-inputs/run36-{pdf-fonts.go,extract-fonts.py}`; fixtures:
`crates/d2-rs-export/tests/fixtures/pdf-fonts-fpdf.json`. An initial fixture
extraction boundary included the following resource object; corrected before
qualification. Final comparison includes precisely seven raw objects per font.
Go toolchain verified **1.27.0**. Latest aliases updated. CLI capture **5,533,882
bytes**, largest **723,311**; all streaming budgets pass.

Next: the same five PDFs' outer FPDF serialization/page streams/resource IDs
and metadata. Four PPTX rows remain explicitly blocked by source/release
AppVersion; keep native v0.9.0. Broader subtree packaging/stdio and standalone
PNG eXIf remain supplementary gaps. External plugins remain excluded.

Cleanup retired **4,495 files**, recovering **891.87 MiB allocated / 876.95 MiB apparent**:
obsolete debug rlib/rmeta files and idle Go actions plus the disposable edit
script. Cargo locks/scoped lsof pass. Current binaries, corpus inputs/outputs,
all captures and negative witnesses remain; golden capture hashes verify.
Ledger: `run36-cleanup.json`. No bulk moved; compression deferred per brief
Update 3 (`.reference/` below 10 GiB allocated).

Disk allocated/apparent GiB: checkout **18.51/30.77**,
`.reference` **7.11/18.24**, `target` **9.41/10.37**;
**96.13 GiB free**. All ceilings pass, including hidden paths/.git
and every apparent hard-linked path. Audit: `run36-disk.json` and raw du/df
files. Gross cleanup savings are distinct from builds' net storage change.

### Run 35 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the complete brief through Update 11.
Closed the measured harness gap in `TestCLI_E2E/validate-against-correct-d2`.
No Rust behavior change or native rebuild; no commits by this run; inherited
`29` preserved. Existing run-34 changes were committed separately as `9b8106e`.

- Pinned Go `validateCmd` writes success text directly to process stdout
  (`d2cli/validate.go:37`), bypassing mocked `ms.Stdout`. The instrumentation-only
  hook tees that same write to the capture sink, retaining the original process
  output. Ordinary execution is unchanged when capture is disabled.
- Go and Rust now sequentially use the same fresh per-case input directory;
  stale directories are rejected. All **158 raw stdout bytes**, including
  the absolute input path, match Go process output, Go capture and Rust capture.
  No path/text normalization, comparator relaxation, exclusion or deadline change.
- This is **one newly qualified existing native case**, not a claimed native
  implementation fix. All 70 cases and 61 goldens remain inventoried.

| scoreboard tier | before → run 35 | regressions |
|---|---:|---:|
| CLI assertions | 66/70 → 66/70 | **0** |
| CLI strict complete artifacts | 56/70 → **57/70** | **0** |
| CLI retained goldens | 46/61 → 46/61 | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five scoreboards are fresh; exporter assertions **2/2**. **13 Python
harness tests and three top-level Go hook tests pass** (plus ordinary/capture
subtests), covering complete stdout, absent/changed bytes, idempotent hook
installation, reusable clean input paths and the existing diagnostic adapter.
The first Python run had a test-only whitespace error; corrected and rerun.
Go formatting and `git diff --check` pass. Raster harness source unchanged.

Evidence: `run35-*-final.json`, five deltas, `run35-validation.json`, test/run
logs and `report-run35.md`. Latest aliases updated. CLI capture **5,514,383
bytes**, largest **723,311**; sketch/exporter captures also pass the unchanged
streaming 1 GiB total / 128 MiB per file / 20 GiB free guards. Hook manifests
and golden-tier capture hashes verify against the retained warm binaries.

Next: the five PDF strict artifact rows (`internal_linked_pdf`, `renamed-board`,
`no-nav-pdf`, `theme-pdf`, `stdout_pdf`) still differ in serialization/fonts/
metadata. Four PPTX strict rows retain the explicitly classified source/release
AppVersion difference. Broader named-subtree packaging/stdio and standalone
PNG eXIf remain open supplementary contracts. Full D2/API parity is incomplete;
external plugins remain excluded.

Cleanup recovered **383.41 MiB allocated** from idle Go
actions and disposable files; current binaries, corpus inputs/outputs and all
negative witnesses remain. Cargo locks and scoped lsof passed. A staging check
stopped on retained nonempty failure evidence after cache deletion; the recovery
ledger records the measured directory delta, without per-file cache hashes or
an unmeasured apparent-savings claim (`run35-cleanup.json`). No bulk moved;
compression deferred per brief Update 3.

Disk allocated/apparent GiB: checkout **18.94/31.21**,
`.reference` **7.09/18.22**, `target`
**9.86/10.83**; **97.27 GiB free**. All ceilings
pass, including hidden paths/.git and every apparent hard-linked path.
Audit: `run35-disk.json` and final raw du/df files.

### Run 34 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the complete brief, including the
new **Update 11**. Baseline `d99b711`; no commits; inherited `29` preserved.
Selected the shared Go encoders affecting the five failing PDF cases
(`internal_linked_pdf`, `renamed-board`, `no-nav-pdf`, `theme-pdf`, `stdout_pdf`)
and four PPTX cases (`how_to_solve_problems_pptx`, `pptx-theme-overrides`,
`no-nav-pptx`, `stdout_pptx`).

- Shared native PNG encoding now follows Go 1.27.0's adaptive filter tie order,
  opaque-NRGBA RGB selection, BestSpeed zlib and buffered IDAT writes. Reuses
  `d2-rs-goflate`; no runtime oracle. Standalone PNG IDAT chunks improve
  **0/12 → 12/12 exact**; all twelve decoded outputs remain exact. Complete
  PNG bytes remain different: Go adds an `eXIf` chunk via `lib/png`, which
  native standalone output still lacks. Embedded PPTX PNGs improve
  **0/8 → 8/8 complete byte matches**.
- FPDF filters RGBA before splitting color and alpha; each resulting plane
  is compressed with BestSpeed. All **eight RGB and eight alpha streams**
  improve **0/8 → 8/8 byte matches**, preserving complete decoded pixels,
  dimensions and image inventories on all eight PDF pages.
- Native ZIP assembly follows Go's level-5 DEFLATE, zero generated timestamps,
  header versions, descriptors and raw template-member copies. Across all
  four decks, exact compressed members improve **144/196 → 192/196**;
  decompressed members **184/196 → 192/196**. The remaining four member
  differences are exactly AppVersion. The CLI scoreboard explicitly reports
  development-source `0.8.1` versus release `0.9.0`, with the pinned-source
  reason; artifacts stay non-passing. Native release version stays v0.9.0.

Source: `d2renderers/d2raster/raster.go:563–585`, Go 1.27.0 `image/png/writer.go`,
`bufio.Writer`, `archive/zip/{writer,register}.go`; fpdf v0.12.0
`png.go:170–246`, `xcompr.go:25–44`. Mapping/notices updated in the exporter.

| scoreboard tier | before → run 34 | regressions |
|---|---:|---:|
| CLI assertions / strict artifacts / goldens | 66/70 / 56/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five scoreboards are fresh; exporter assertions **2/2**. No match verdict,
normalization, exclusion or deadline was relaxed; **no whole-artifact score
improvement is claimed**. PDF serialization/fonts/metadata still differ.
Full D2/API parity remains incomplete; external plugins remain excluded.

**436 tests pass: 209 exporter + 216 CLI + 11 CLI-harness tests.** A further
ZIP test rerun passes after reviewing ZIP64 header ordering. Six Go-derived
PNG/plane cases cover opacity, alpha, filters and multiple IDAT chunks; a full
Go ZIP fixture covers template raw copies, empty entries and Unicode names.
The generator rejects any toolchain except Go 1.27.0 (the initial accidental
system-Go fixture attempt was corrected). Raster harness source unchanged;
PPTX pixel evidence compares both RGB and alpha through its existing complete
PNG gate, after validating every original image. Diagnostic CLI/adapter builds,
focused rustfmt and diff checks pass.

Evidence: `run34-*-final.json`, five deltas, `run34-encoding-analysis.json`,
`run34-validation.json`, test/build logs and `report-run34.md`. Generator:
`witness-inputs/run34-encoders.go`; checked-in fixtures:
`png-encoding-go127.json`, `zip-encoding-go127.zip`. CLI capture **5,501,480
bytes**, largest **723,311**; streaming 1 GiB total / 128 MiB file / 20 GiB
free guards pass. Shared latest aliases updated.

Next: validation's direct-process-stdout capture gap is the remaining likely
whole-case flip. PDF complete serialization/fonts/metadata, standalone PNG's
missing `lib/png` eXIf metadata, and broader named-subtree packaging/stdio remain.
PPTX source/release metadata stays visibly classified, not forged or normalized.

Cleanup retired **5,249 files**, recovering **557.21 MiB allocated / 540.06 MiB apparent**: idle Go actions, superseded export test/library variants, earlier successful capture copies and disposable image-plane staging. Cargo locks/scoped lsof pass. Original corpus input/output hashes verify after cleanup (`run34-closure-validation.json`); replacement paths/hashes/rebuild commands are in `run34-cleanup.json`. No bulk moved; compression deferred per brief Update 3.

Disk allocated/apparent GiB: checkout **18.93/31.20**, `.reference` **7.08/18.21**, `target` **9.86/10.83**; **98.42 GiB free**. All ceilings pass, including hidden paths/.git and every apparent hard-linked path. Audit: `run34-disk.json` and final raw du/df files.

### Run 33 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the full brief through Update 10.
Baseline `a73ecea`; no commits; inherited untracked `29` preserved.
Selected the retained run-28 named-wildcard output failure after confirming
all applicable upstream assertions already pass and strict PDF/PPTX bytes
remain the larger incidental-encoding cluster.

- Nonanimated SVG/PNG `--target=board.*` now writes the selected board and
  every renderable descendant through the shared recursive file traversal.
  Previously only default/root wildcards and named animated SVGs entered that
  path, so a named wildcard silently emitted a single file. Canonical target
  resolution supports shorthand and quoted names. Source: `d2cli/main.go:368–385,
  539–549,859–947` (noChildren, GetBoard and renderWithPNGEncoder).
- Go-derived fixtures cover nested/shorthand targets, quoted names, mixed
  collections, folder-only parents and leaves, with both flag spellings.
  All **five SVG and five PNG witness variants improve 0/5 → 5/5**:
  complete inventories of **nine SVG files and nine PNG files** match;
  SVG bytes and every decoded PNG pixel are exact. PNG uses the existing
  `compare_png_outputs` completeness/decoding gate. These are supplementary
  source witnesses; they do not inflate upstream scoreboard counts.

| scoreboard tier | before → run 33 | regressions |
|---|---:|---:|
| CLI assertions / strict artifacts / goldens | 66/70 / 56/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five scoreboards are fresh; exporter assertions remain **2/2**. No
comparator, normalization, exclusions or deadlines changed. First CLI capture
hit GIF fixture 1050's existing 90-second deadline during concurrent builds;
its failure and delta remain (`run33-cli-initial.json`, capture `run33-final/`).
The full isolated rerun (`run33-qualified/`) has zero regressions.

**216 CLI tests pass** (`cargo test --profile diagnostic -p d2-rs-cli --bin
 d2-rs`). Diagnostic CLI and warm golden adapter builds, rustfmt and
`git diff --check` pass. An unnecessary oracle-profile adapter attempt was
interrupted; it produced no executable, and its idle codegen was removed.
Raster harness source unchanged. Evidence: `run33-*-final.json`, five deltas,
`run33-subtree-{before,after}.json`, `run33-validation.json`, test/build logs,
and `report-run33.md`. Generator: `witness-inputs/run33-subtree-output.py`;
fixture: `crates/d2-rs-cli/tests/fixtures/subtree-output-source-v090.json`.
Qualified CLI capture **5,110,533 bytes**, largest **723,311**; streaming
1 GiB total / 128 MiB per file / 20 GiB free guards pass. Compact before/after
witness trees (308 KiB allocated) and the failed capture remain; latest aliases updated.

Next: PDF/PPTX representation/metadata, validation's direct-process-stdout
capture gap, and named-subtree packaging/stdio beyond these SVG/PNG file
witnesses. TXT was scoped out after pinned Go failed fresh output-parent
creation; no TXT compatibility gain is claimed. Full D2/API parity remains
incomplete; external plugins remain excluded.

Cleanup retired **4,515 files**, recovering **600.14 MiB allocated / 572.02 MiB apparent**:
idle Go action cache, Cargo codegen (including the interrupted build) and
superseded fixture/partial witness staging. Cargo locks and scoped lsof pass;
no active files or corpus inputs/expectations retired. Original corpus hashes
verify (`run33-closure-validation.json`); ledger/replacements:
`run33-cleanup.json`. No bulk moved. Compression deferred per brief Update 3
(.reference below 10 GiB allocated).

Disk allocated/apparent GiB: checkout **18.77/31.04**, `.reference` **7.06/18.19**, `target` **9.73/10.69**; **99.95 GiB free**. All ceilings pass, including hidden paths/.git and every apparent hard-linked path. Audit: `run33-disk.json` and final raw du/df files.

### Run 32 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the entire brief through Update 10.
Targeted the five PDF artifact rows: `internal_linked_pdf`, `renamed-board`,
`no-nav-pdf`, `theme-pdf` and `stdout_pdf`. No commits; inherited `29` preserved.
The inherited run-31 changes are now in baseline commit `d5a4f0c`.

- Internal board and breadcrumb links now use `/XYZ 0 <target page height>
  null`, opening the destination at its top while preserving zoom. The full
  destination height includes its navigation band and is calculated per page.
  Previously `/Fit` changed the zoom. Source: `lib/pdf/pdf.go:149–150,184–187`
  and pinned fpdf `fpdf.go:4304–4323`.
- Separator stroke components now use FPDF's three-decimal precision
  (`rgbColorValue`, `fpdf.go:951–971`). All **nine annotations** (including
  external URLs) match pinned Go. Matching link-page inventories improve
  **2/8 → 8/8**; separator colors **0/8 → 8/8**. Page geometry and embedded
  RGB/alpha pixels remain exact on **8/8** pages.
- Complete Poppler captures of all eight pages at 72 dpi match pinned Go
  pixel-for-pixel, both before and after. Capture completeness is checked
  against each PDF's page count, then the existing `compare_png_outputs`
  gate validates every output. This is bounded evidence for these five cases,
  not full PDF/API parity.

| scoreboard tier | before → run 32 | regressions |
|---|---:|---:|
| CLI assertions / strict artifacts / goldens | 66/70 / 56/70 / 46/61 → unchanged | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five scoreboards ran freshly; exporter assertions remain **2/2**. Strict
PDF/PPTX byte gaps remain visible. No comparator, normalization or exclusion
changed, and no complete-artifact score improvement is claimed.

**206 exporter + 215 CLI = 421 tests pass**. The source-derived PDF fixture
covers unequal page heights, navigation on/off and forward/backward/breadcrumb
links. Diagnostic CLI/adapter builds, rustfmt and `git diff --check` pass.
Raster harness source unchanged. Evidence: `run32-*-final.json`, five deltas,
`run32-pdf-{content,renders}.json`, `run32-validation.json`, test/build logs,
and `report-run32.md`. Generator: `witness-inputs/run32-pdf-navigation.go`;
fixture: `crates/d2-rs-export/tests/fixtures/pdf-navigation-source-v090.json`.
Fresh CLI capture: **5,122,855 bytes**, largest **723,311**; 1 GiB total /
128 MiB per file / 20 GiB free streaming guards pass. Latest aliases updated.

Next: strict PDF/PPTX representation/metadata gaps remain; all measured PDF
link/page/image contracts and the four decks' XML contracts (except version)
now agree with Go. Validation's direct-process-stdout capture and named-wildcard
child inventories remain open (run 30/run 28). Full D2/API parity is incomplete.

Cleanup removed **4,352 files**, recovering **388.06 MiB allocated / 373.43 MiB apparent**: idle Go action cache and completed disposable PDF/render staging. Cargo locks and scoped lsof pass. All captured PDFs, source generators, fixtures, comparisons, corpus inputs/expectations and current warm executables remain. Original hashes verify 328 e2e inputs, 3,120 pinned input/output blobs, 930 bound language expectations, 61 CLI and 58 sketch/exporter goldens. Ledger: `run32-cleanup.json`; checks: `run32-closure-validation.json`. No bulk moved. Compression deferred per brief Update 3 (.reference below 10 GiB allocated).

Disk allocated/apparent GiB: checkout **18.75/31.02**, `.reference` **7.04/18.17**, `target` **9.73/10.69**; **102.23 GiB free**. All ceilings pass, including hidden paths/.git and every apparent hard-linked path. Audit: `run32-disk.json` and raw du/df files.

### Run 31 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the complete brief through Update 10.
Selected the shared PPTX XML gaps in `how_to_solve_problems_pptx`,
`pptx-theme-overrides`, `no-nav-pptx` and `stdout_pptx`. No commits;
inherited untracked `29` preserved. Baseline source: `dd0782c`.

- Replaced abbreviated XML with the complete native translation of Go's
  content types, presentation, root/presentation relationships, slides and
  slide relationships. All eleven layout declarations, nine text levels,
  guides, header properties and link hit-region/interaction attributes now
  follow `lib/pptx/templates/` and `pptx.go:360–516`.
- Link shape IDs start at zero, relationship IDs at `link1`; link relationships
  precede navigation relationships. Archive member order follows
  `pptx.go:203–289`. No-navigation fitting now uses the same IMAGE_WIDTH
  threshold and float/truncation sequence as `AddSlide` (`106–158`).
- Across the four original cases, **116/152 → 148/152 exact XML members**.
  Every ZIP member name/order and all eight embedded PNG pixel arrays match.
  The remaining four XML differences are solely AppVersion: the pinned Go
  development library writes `0.8.1`, the native CLI writes `0.9.0`.

| scoreboard tier | before → run 31 | regressions |
|---|---:|---:|
| CLI original assertions | 66/70 → 66/70 | **0** |
| CLI strict complete artifacts | 56/70 → 56/70 | **0** |
| CLI retained goldens | 46/61 → 46/61 | **0** |
| e2e SVG / expected errors | 933 / 11 → unchanged | **0** |
| language JSON / editor source+errors | 915 / 445 → unchanged | **0** |
| sketch / exporter goldens | 38/39 / 19/19 → unchanged | **0** |

All five scoreboards ran freshly; exporter assertions remain **2/2**. The
strict CLI comparator and all exclusions are unchanged: **no complete-artifact
or golden score increase is claimed**. PNG/ZIP encoding and version metadata
still prevent strict PPTX-byte matches. Five PDF cases also remain strict
artifact mismatches; full D2/API parity remains incomplete.

**205 exporter + 215 CLI = 420 tests pass**. A complete-XML Go fixture test
covers navigation on/off, internal/external links, escaping and a 170×100
image that exercises the no-navigation width threshold. It fails against the
pre-fix writer. One intervening test run used mismatched old/new witness
sizes after a queued build; its failed log is retained, and the fixed
source/fixture suite passes in `run31-export-qualified-test.log`.
Diagnostic CLI/adapter builds, changed-block rustfmt and `git diff --check`
pass. Raster harness unchanged.

Evidence: `run31-{cli,e2e,language,sketch,exporter}-final.json`, five deltas,
`run31-pptx-content.json`, `run31-validation.json`, and `report-run31.md`
under `.reference/codex-runs/`. Original Go fixture generator:
`witness-inputs/run31-pptx-parts.go`; complete XML expectations are checked in
under `crates/d2-rs-export/tests/fixtures/pptx-parts/`. CLI capture is
`run31-final/`, **5,122,919 bytes**, largest **723,311**; 1 GiB total / 128 MiB
per file / 20 GiB free guards pass. Shared latest aliases updated.

Next: the five PDF artifact differences; PPTX's remaining version/encoding
boundary is now isolated. Validation's direct-process-stdout capture gap and
named-wildcard child inventories remain as documented in run 30/run 28.

Cleanup retired **4,306 files**, recovering **387.10 MiB allocated / 372.50 MiB apparent**: idle Go actions and completed translation/source staging. Cargo locks and scoped lsof pass; current warm executables and the latest WASM qualification's native baseline remain. No corpus inputs/expectations or captures retired; original hashes verify 328 e2e inputs, 3,120 pinned input/output Git blobs, 930 bound language expectations, 61 CLI and 58 sketch/exporter goldens. Fresh tier capture hashes verify. Ledger: `run31-cleanup.json`; checks: `run31-closure-validation.json`. No bulk moved. Compression deferred per brief Update 3 (.reference below 10 GiB allocated).

Disk allocated/apparent GiB: checkout **18.72/30.99**, `.reference` **7.00/18.14**, `target` **9.73/10.69**; **104.90 GiB free**. All ceilings pass, including hidden paths/.git and every apparent hard-linked path. Net checkout change **+16.91/+14.43 MiB allocated/apparent**, distinct from gross cleanup savings. Audit: `run31-disk.json` and raw du/df files.

### Run 30 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the complete brief through Update
10. Continued the PPTX queue, then closed the remaining applicable CLI assertion
failures. No commits; inherited untracked `29` preserved.

- `stdout_pptx`: presentation title/subject now follow the output basename or
  input basename for stdout (`stdin` for stdin-to-stdout); creator uses the OS
  account. Explicit per-presentation core/app properties retain all ordered
  board IDs and counts. Source: `d2cli/main.go:630–644`,
  `lib/pptx/pptx.go:269–302`, `templates/{core,app}.xml`. Multiboard PPTX stdout
  now returns the package instead of rejecting stdout or creating `-`.
- `incompatible-animation`: reject non-SVG/GIF file output before source I/O,
  preserving Go's stdout exception (`d2cli/main.go:252–258`). The process
  adapter now removes one stderr transport prefix per line, preserving blank
  lines, authored prefixes and whitespace. A Go test checks the complete error
  through xmain; the original failed capture remains.
- `fmt-check-unformatted`: count every differing file, preserving all input
  bytes and the singular/plural diagnostic (`d2cli/fmt.go:26–66`).

| scoreboard tier | before | run 30 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 63/70 | **66/70** | **0** |
| CLI complete artifacts | 54/70 | **56/70** | **0** |
| CLI retained goldens | 46/61 | **46/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor source+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

All five scoreboards ran freshly; exporter assertions remain **2/2**. No byte
comparator, normalization or exclusion changed. CLI's four non-passing assertion
rows are the excluded external-layout case, two unsupported injected Go writers
and one upstream skip. Full D2/API parity remains incomplete.

**215 CLI + 204 exporter + 33 harness + one Go adapter = 453 tests pass**.
Diagnostic CLI/adapter builds, changed-block rustfmt and `git diff --check`
pass. Raster harness unchanged. Complete native core/app XML matches pinned Go
fixtures, including escaping. Four reduced file/stdout/stdin/multiboard controls
fail before and match Go's metadata, complete ZIP inventories and decoded PNGs
after (`run30-pptx-witness.json`). This does not claim complete PPTX-byte parity;
Windows account lookup is implemented but not runtime-qualified here.

Evidence: `.reference/codex-runs/run30-{cli,e2e,language,sketch,exporter}-final.json`,
five deltas, `run30-validation.json`, `run30-pptx-remaining.json`, and
`report-run30.md`. Final CLI capture is `run30-qualified/`, **5,107,008 bytes**,
largest **723,311**; 1 GiB total / 128 MiB file / 20 GiB free streaming guards
passed without exhaustion or timeout. The first `run30-final/` capture and
`run30-cli-initial.json` preserve the multiline adapter failure. Latest aliases
and the shared score binary were updated after all deltas passed.

Next: five PDF and four PPTX strict artifact differences. All four PPTX core
parts and embedded PNG pixels now match; generated XML and encoding differences
remain inventoried. The validation artifact mismatch is a harness boundary:
Go validate prints directly to process stdout outside mocked ms.Stdout
(`d2cli/validate.go:37`); native success output must be preserved. Named-wildcard
child omission remains at `run28-wildcard-gap.json`. No plugin work is required.

Cleanup retired **4,311 files**, recovering **637.01 MiB allocated / 622.40 MiB apparent**: idle Go actions, superseded CLI/export test executables, redundant score aliases and the completed metadata control deck. Current warm executables, all corpus inputs/expectations and negative witnesses remain. Cargo locks/scoped lsof pass; no bulk moved. Ledger/replacements: `run30-cleanup.json`; original hashes and current tier inventories verify in `run30-closure-validation.json`. Compression deferred per brief Update 3 (.reference below 10 GiB allocated).

Disk allocated/apparent GiB: checkout **18.70/30.98**, `.reference` **6.99/18.13**, `target` **9.73/10.69**; **102.62 GiB free**. All ceilings pass, including hidden directories/.git and every apparent hard-linked path. Net checkout change **-41.29/-43.99 MiB allocated/apparent**, distinct from gross cleanup savings. Audit: `run30-disk.json` and raw du/df files.

### Run 29 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the full brief through **Update
10**. Fixed `TestCLI_E2E/with-font`: measurement already used the custom face,
but SVG embedding lost its bytes and subsetted the bundled font. Native
`CustomFonts` now carries per-render primary/mono faces through ordinary and
animated d2svg rendering, using the existing exact UTF8CutFont/Sfnt2Woff path.
Omitted styles follow SourceSansPro fallbacks, including customMono, without
mutating bundled fonts. Source: `d2cli/main.go:1117–1200` and
`d2fonts_common.go:38–61,129–209`; CLI/text SOURCE.md map the change.

| scoreboard tier | before | run 29 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 62/70 | **63/70** | **0** |
| CLI complete artifacts | 53/70 | **54/70** | **0** |
| CLI retained goldens | 45/61 | **46/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor source+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

All five scoreboards ran freshly; sketch/exporter use the rebuilt diagnostic
adapter. Exporter assertions remain **2/2**. No comparator, exclusion or
normalization changed. The original font witness plus reduced regular, mono
and animated multiboard controls match complete normalized Go SVGs; all four
fail with the previous binary. Native tests cover face selection, omitted
styles and isolation between renders. **211 CLI + 11 font + 33 harness = 255
tests pass**. Diagnostic CLI/adapter builds, changed-block rustfmt and
`git diff --check` pass. Raster harness unchanged.

Evidence: `.reference/codex-runs/run29-{cli,e2e,language,sketch,exporter}-final.json`,
five deltas, `run29-font-witness.json`, `witness-inputs/run29-font-*.d2`,
`run29-validation.json`, and `report-run29.md`. CLI capture **5,126,181 bytes**,
largest **723,311**; streaming 1 GiB total / 128 MiB file / 20 GiB free guards
passed without exhaustion or timeout. Latest aliases updated only after deltas
passed. No commits; inherited untracked `29` preserved.

Next: PDF/PPTX complete-artifact differences and the isolated assertions
`incompatible-animation`, `stdout_pptx`, `fmt-check-unformatted`. Three PPTX
cases share complete ZIP inventories but differ in generated XML and embedded
PNG bytes (`run29-pptx-remaining.json`); the title gap still maps to
`d2cli/main.go:630–644` / `lib/pptx/pptx.go:269–288`. Named-wildcard child
output omission remains at `run28-wildcard-gap.json`. External layout-extra-args
stays inventoried and out of scope. Full D2/API parity remains incomplete.

Cleanup retired **4,671 files**, recovering **434.18 MiB allocated / 418.46 MiB apparent**: idle Go actions, the superseded oracle golden-tier adapter, run-28's successful CLI capture and completed formatting scratch. Cargo locks and scoped lsof checks passed. Current diagnostic adapters, all corpus inputs/expectations, compact negative font witnesses and the run-28 watch failure remain. No bulk moved. Original hashes verify 328 e2e inputs, 1,878 e2e outputs, 930 bound language expectations, 61 CLI and 58 sketch/exporter goldens; 403 fresh tier capture files and 3,120 pinned Git blobs also verify. Ledgers/replacements: `run29-cleanup.json`, `run29-closure-validation.json`. Compression deferred per brief Update 3 (.reference below 10 GiB allocated).

Disk allocated/apparent GiB: checkout **18.74/31.02**, `.reference` **6.96/18.11**, `target` **9.79/10.76**; **108.34 GiB free**. All ceilings pass, including hidden paths/.git and every apparent hard-linked path. Net checkout change **+23.09/+21.27 MiB allocated/apparent** is separate from gross cleanup savings. Audit: `run29-disk.json` and raw du/df files.

### Run 28 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the entire brief through **Update
10**. Fixed the live `target-root` SVG golden. Pinned
`d2cli/main.go:368–377,542–545` clears descendants before hashing an explicit
empty/named target. Rust now follows that selection boundary; internal document
traversal explicitly retains subtrees. Reduced root/named controls match complete
Go SVGs; both flag forms and wildcard hashes have a native regression.

A repeat capture exposed `watch-regular` resetting its WebSocket handshake.
Accepted macOS sockets can inherit the listener's nonblocking flag. The handler
now clears it while reading HTTP headers, following `d2cli/watch.go:475–486`'s
HTTP server; WebSockets enable it again after the handshake. The delayed-header
regression fails before the fix and passes after. The failed capture and delta
remain at `run28-qualified/` and `run28-cli-watch-negative-delta.json`.

| scoreboard tier | before | run 28 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 61/70 | **62/70** | **0** |
| CLI complete artifacts | 52/70 | **53/70** | **0** |
| CLI retained goldens | 44/61 | **45/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor source+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

CLI/e2e/language ran freshly. Sketch/exporter rescore complete, hash-verified
run-25 captures with their unchanged adapter; changed paths are not invoked.
Exporter assertions stay **2/2**. No comparator, exclusion or normalization
changed. **210 CLI + 33 harness tests pass**; diagnostic build, changed-block
rustfmt and `git diff --check` pass. Raster harness unchanged. Final CLI capture:
**5,129,133 bytes**, largest **723,311**; streaming 1 GiB total / 128 MiB per
file / 20 GiB free guards passed without exhaustion or timeout. Evidence:
`.reference/codex-runs/run28-{cli,e2e}-verified.json`, language/sketch/exporter
final reports, five deltas, `run28-validation.json`, and `report-run28.md`.

Next: `with-font` has correct geometry/hash but embeds bundled rather than custom
font bytes (`d2cli/main.go:1117–1200`, `d2fonts_common.go:38–61`,
`d2svg/fonts.rs:258–270`). PDF/PPTX have structural/content differences beyond
the known PPTX title gap. Remaining assertions: incompatible-animation,
stdout_pptx, fmt-check-unformatted. A reduced `layers.one.*` CLI case also
omits child output files; source plus complete Go/Rust inventories remain in
`run28-wildcard-gap.json` and `witness-outputs/run28-wildcard/`. External
layout-extra-args remains inventoried and out of scope. Full D2/API parity remains
incomplete. No commits; untracked `29` preserved.

Cleanup retired **4,939 files**, recovering **437.47 MiB allocated / 420.99 MiB apparent**: idle Go actions, a redundant idle diagnostic executable (identical `d2-rs-traces` retained), superseded successful captures and completed scratch. Cargo locks and scoped lsof checks passed. No corpus input/expected output or canonical module was removed; no bulk moved. The failed watch capture remains. Original hashes verify 930 bound language expectations, 61 CLI and 58 sketch/exporter goldens. Ledger/replacements: `run28-cleanup.json`; checks: `run28-closure-validation.json`. Compression deferred per brief Update 3 (.reference below 10 GiB allocated).

Disk allocated/apparent GiB: checkout **18.72/31.00**, `.reference` **6.95/18.10**, `target` **9.78/10.74**; **107.57 GiB free**. All ceilings pass, including hidden directories/.git and every apparent hard-linked path. Net checkout change **-41.17/-43.37 MiB allocated/apparent** is separate from gross cleanup savings. Audit: `run28-disk.json` and raw du/df files.

### Run 27 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the full brief through **Update
10**. Chose `multiboard/life_index_d2`: five live SVG goldens shared a single
file-resolution failure. No commits by this run; untracked `29` preserved.

Pinned `d2cli/main.go:231–244` chooses the default output from the original
input argument, then resolves directory inputs to `index.d2`. Rust now follows
that order, retaining the original output stem and using the resolved source
for imports and diagnostics. The source map is in the CLI crate's SOURCE.md.
The native regression covers relative imports, explicit/default outputs and
the default multiboard folder. The full upstream case matches all five SVGs:
`index`, `layers/{broker,core,stocks}`, and `scenarios/why`.

| scoreboard tier | before | run 27 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 60/70 | **61/70** | **0** |
| CLI complete artifacts | 51/70 | **52/70** | **0** |
| CLI retained goldens | 39/61 | **44/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor source+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

Fresh Go passes all 69 enabled CLI cases. CLI/e2e/language ran freshly;
sketch/exporter rescore hash-verified, complete run-25 captures with their
unchanged adapter (this change affects document file resolution, which those
adapters do not invoke). Exporter assertions remain **2/2**. No comparator,
normalization or exclusion changed. Strict PDF/PPTX artifacts remain
non-passing; full D2/API parity remains incomplete.

**208 CLI tests + 33 scoreboard harness tests pass.** Diagnostic build,
changed-block rustfmt and `git diff --check` pass. Raster harness unchanged,
not rerun. Final CLI capture: **5,156,126 bytes**, largest **723,311**; streaming
1 GiB total / 128 MiB per file / 20 GiB minimum free guards passed, without
exhaustion or timeout. Evidence: `.reference/codex-runs/run27-{cli,e2e}-qualified.json`,
`run27-{language,sketch,exporter}-final.json`, five deltas, test/build logs,
`run27-validation.json` and `report-run27.md`.

Next: PDF/PPTX artifact differences and remaining CLI assertions (`with-font`,
`incompatible-animation`, `target-root`, `stdout_pptx`,
`fmt-check-unformatted`). The known PPTX title gap maps to
`d2cli/main.go:630–644` / `lib/pptx/pptx.go:269–288`. External
`layout-extra-args` stays inventoried and excluded as a completion gate.

Cleanup retired **4,159 files**, recovering **369.27 MiB allocated /
355.17 MiB apparent**: idle Go actions and completed formatting scratch.
Scoped lsof found no open cache files; no corpus input, expected output,
module or executable was removed, and no bulk moved. Original hashes verify
328 e2e inputs, 1,878 e2e expectations, 930 bound language expectations,
61 CLI and 58 sketch/exporter goldens after cleanup. Evidence:
`run27-cleanup.json` and `run27-closure-validation.json`. Compression deferred
per brief Update 3 (.reference below 10 GiB allocated).

Disk allocated/apparent GiB: checkout **18.76/31.04**, `.reference` **6.94/18.09**, `target` **9.83/10.80**; **109.86 GiB free**. All ceilings pass, including hidden directories/.git and every apparent hard-linked path. Net checkout growth **8.20/7.26 MiB** is separate from gross cleanup savings. Audit: `run27-disk.json` and raw du/df files.

### Run 26 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the complete brief through
**Update 10**. Chose the shared GIF artifact cluster: `how_to_solve_problems_gif`,
`animated-gif`, and `one-layer-gif`. All three now pass. No commits;
untracked `29` preserved.

Rust used NeuQuant and quantized the padded canvas. Pinned Go selects each
source frame's palette with go-quantize's median cut, dithers with exact
Floyd–Steinberg error arithmetic, then normalizes white and centers the indexed
frame. The native translation preserves occupied-slot traversal, median axis
ties and first-mode selection. Shared GIF packaging now follows that order.
Static CLI boards also follow the requested interval's sample/delay schedule;
single-frame output omits the loop extension. Source: `lib/xgif/xgif.go:160–232,
506–548,887–978,1592–1653`, `lib/xgif/encode.go:224–245,396–406`,
`d2cli/raster.go:633–666`, and pinned go-quantize `ae555eb2afa4`
`quantize/{mediancut,bucket}.go`. Source maps and the complete BSD notice are
recorded in the export crate and root notices.

| scoreboard tier | before | run 26 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 59/70 | **60/70** | **0** |
| CLI complete artifacts | 48/70 | **51/70** | **0** |
| CLI retained goldens | 39/61 | **39/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor text+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

Fresh CLI capture reproduces all 69 enabled Go cases. E2e and language also
ran freshly. Sketch/exporter rescore complete, hash-verified run-25 captures
with their unchanged adapter; those SVG/target JSON paths do not invoke GIF.
Exporter assertions remain **2/2**. No comparator, exclusion or normalization
changed. The CLI gate compares every decoded GIF frame, canvas, delay and loop
value. The historical stale GIF golden remains non-passing.

**410 native tests (203 export + 207 CLI) and 33 harness tests pass.** A small
gradient/centering regression matches pinned xgif's complete decoded RGB hash;
its Go reproducer is `witness-inputs/run26-quantize.go`. Static source controls
check both single-frame loop/delay and repeated-board sampling. Diagnostic
builds, changed-block rustfmt and `git diff --check` pass. Raster harness
unchanged, not rerun. Final capture: **5,117,996 bytes**, largest **723,311**;
1 GiB total / 128 MiB per file / 20 GiB free streaming guards passed.
Evidence: `.reference/codex-runs/run26-{cli,e2e}-qualified.json`,
`run26-{language,sketch,exporter}-final.json`, five deltas,
`run26-validation.json`, and `report-run26.md`. Latest JSON aliases replaced
atomically; negative run-25 CLI captures remain.

Next: the PDF/PPTX artifact cluster and remaining assertion failures. The
known PPTX title/metadata gap still maps to `d2cli/main.go:630–644` and
`lib/pptx/pptx.go:269–288`; `multiboard/life_index_d2` has five live SVG goldens
behind one assertion failure. Other failures: `with-font`,
`incompatible-animation`, `target-root`, `stdout_pptx`, and
`fmt-check-unformatted`. External `layout-extra-args` stays inventoried and
excluded as a completion gate. Strict PDF/PPTX bytes remain non-passing;
these finite matches do not establish full D2/API parity.

Cleanup: `run26-cleanup.json` records **4,920 files** retired and
**517.98 MiB allocated / 501.51 MiB apparent** recovered. Removed superseded
export compilation variants, idle Go actions and duplicate intermediate
captures/reports after Cargo locks and scoped lsof checks. Current warm/oracle
executables, module cache, negative baselines and complete final captures
remain; no bulk moved. Post-cleanup checks verify **328 original e2e input
hashes, 1,878 expected outputs, 930 bound language expectations, 61 CLI and
58 sketch/exporter goldens, and 4,495 pinned Git blobs**
(`run26-closure-validation.json`). Compression deferred per brief Update 3
(.reference below 10 GiB allocated).

Disk: checkout **18.75 / 31.04 GiB allocated / apparent**, `.reference`
**6.93 / 18.08 GiB**, `target` **9.83 / 10.80 GiB**; free **110.63 GiB**.
All ceilings pass. Hidden directories/.git and every apparent hard-linked path
are included. Net checkout growth: **119.74 / 118.48 MiB allocated / apparent**,
distinct from gross cleanup savings. Audit: `run26-disk.json` and raw du/df
files. Target has limited headroom; check/prune idle variants before another
artifact-heavy build.

### Run 25 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the full brief through **Update
10**. Chose `sequence-layer` and `sequence-spread-layer` by their shared
scoreboard impact. Both now pass. No commits; untracked `29` preserved.

`d2compiler/compile.go:59–60` sorts objects and edges only on the document
graph. Child boards are compiled directly by `compileBoardsField:199`, so
they retain `compileMap` traversal order. Rust incorrectly sorted child
boards too. Keeping both their object and edge order fixes SVG message
ordering and target hashes while preserving the matching geometry. The
native regression distinguishes document and child-board order; reduced
controls match pinned Go SVGs and target hashes (`run25-witness.json`).
The original imported/spread sequence cases cover the complete CLI contract.
Source mapping is recorded in `crates/d2-rs-language/SOURCE.md`.

| scoreboard tier | before | run 25 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 57/70 | **59/70** | **0** |
| CLI complete artifacts | 46/70 | **48/70** | **0** |
| CLI retained goldens | 35/61 | **39/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor text+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

All five scoreboards are fresh runs; exporter assertions remain **2/2**.
Go passes all 69 enabled CLI cases. No comparator, exclusion or normalization
changed. Strict PDF/PPTX and other remaining artifact differences are still
non-passing; full D2/API parity remains incomplete.

**265 native tests (206 CLI + 59 language) and 33 harness tests pass.**
Diagnostic and oracle adapter builds, changed-block rustfmt and
`git diff --check` pass. Raster harness unchanged, not rerun. Evidence:
`.reference/codex-runs/run25-{cli-final,e2e-final,language-final,sketch-final,exporter-final}.json`,
their five deltas, test/build logs, `run25-validation.json`, and
`report-run25.md`. Final CLI capture is **4,954,993 bytes**, largest
**722,768 bytes**. Capture guards: 1 GiB total / 128 MiB per file / 20 GiB
minimum free; no exhaustion or timeout. Latest aliases replaced atomically.

Next: investigate the remaining shared PDF/PPTX/GIF artifact clusters before
the isolated assertion failures (`with-font`, `incompatible-animation`,
`target-root`, `multiboard/life_index_d2`, `one-layer-gif`, `stdout_pptx`,
`fmt-check-unformatted`). The known PPTX title source mapping is
`d2cli/main.go:630–644` / `lib/pptx/pptx.go:269–288`. External
`layout-extra-args` stays inventoried and excluded as a completion gate.

Cleanup retired **9,391 files**, recovering **1565.79 MiB allocated / 1455.51 MiB apparent**. Both Cargo profile locks held and scoped lsof clear. Removed idle compiler temporaries and Go action caches, a superseded adapter and successful intermediate sketch capture, and completed witness scratch. Negative CLI captures remain. No bulk moved; pinned corpus inputs and expected outputs remain. Post-cleanup verification checks 3,120 pinned Git blobs, 328 e2e inputs, 930 bound language hashes, and complete final sketch/exporter capture inventories. The target ceiling was briefly exceeded during adapter builds; parity work stopped for cleanup and fresh passing whole-checkout audits before captures resumed. Ledger/replacement records: `run25-cleanup.json`. Compression deferred per brief Update 3 (.reference below 10 GiB allocated).

Disk: checkout **18.64 / 30.92 GiB allocated / apparent**, `.reference` **6.92 / 18.07 GiB**, `target` **9.72 / 10.69 GiB**; free **112.15 GiB**. All ceilings pass. Hidden paths/.git and every apparent hard-linked path included. Net checkout growth: **78.79 / 75.68 MiB allocated / apparent**, distinct from gross cleanup savings. Audit: `run25-disk.json` and raw du/df files.

### Run 24 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the complete brief through **Update
10**. Chose the four watch artifact failures by scoreboard impact. Fixed
`watch-regular`, `watch-ok-link`, `watch-underscore-link`, and
`watch-nested-layer-link`; the shared link fix also passes `layer-link` and
`board_import`. No commits; inherited work and untracked `29` preserved.

Pinned `d2cli/watch.go:431` calls compile with a selected board and preview
requested. `d2cli/main.go:735–840,859–946` rewrites board links relative to the
complete document output tree, writes child boards before their parent, and
returns the selected preview separately. Rust now follows those boundaries:
links are rewritten before target hashing/rendering, including descendant hash
inputs; watch emits the board tree, returns its preview, and clears only the
selected subtree on navigation. Deep selection starts output at the selected
board's name. SVG document behavior is covered; other watch formats were not
expanded in this change.

| scoreboard tier | before | run 24 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 55/70 | **57/70** | **0** |
| CLI complete artifacts | 40/70 | **46/70** | **0** |
| CLI retained goldens | 30/61 | **35/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor text+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

Fresh Go passes all 69 enabled CLI cases. CLI/e2e/language were freshly run;
sketch/exporter rescore retained run-22 captures with their unchanged,
hash-verified adapter (the document link flag is absent there; no library
rendering changed). Exporter assertions remain **2/2**. Final deltas preserve
all prior matches; no comparator, exclusion or normalization changed. Strict
PDF/PPTX artifact differences remain non-passing. Full D2/API parity is incomplete.

**206 native CLI + 33 harness tests pass.** The new regression checks complete
preview/file correspondence, nested and deep selection, relative links,
stale-subtree removal and sibling preservation. Diagnostic build, changed-file
rustfmt and `git diff --check` pass. Raster harness unchanged, not rerun.
Evidence: `.reference/codex-runs/run24-{cli-final,e2e-sealed,language,sketch,exporter}.json`,
five final deltas, build/test logs, `run24-validation.json` and `report-run24.md`.
Final CLI capture: **5,046,053 bytes**, largest **722,768 bytes**, guarded at
1 GiB total / 128 MiB per file / 20 GiB minimum free; no budget exhaustion or
timeout. Latest aliases replaced atomically. Original corpus hashes verify
after cleanup: 328 e2e inputs, 1,878 outputs, 930 bound language expectations,
53 historical unbound expectations, 61 CLI and 58 sketch/exporter goldens.

Next by common-case impact: `sequence-layer` and `sequence-spread-layer` SVG
mismatches, then the single-case gaps (`with-font`, `incompatible-animation`,
`target-root`, `multiboard/life_index_d2`, `one-layer-gif`, `stdout_pptx`,
`fmt-check-unformatted`). The known PPTX title source mapping remains
`d2cli/main.go:630–644` / `lib/pptx/pptx.go:269–288`. External
`layout-extra-args` remains inventoried and excluded as a completion gate.

Cleanup retired **4,534 files**, recovering **374.50 MiB allocated / 359.23 MiB apparent**. Both Cargo profile locks held and scoped lsof clear. Removed idle Go action cache, the superseded successful initial CLI capture and intermediate e2e reports; retained the final capture, compact deltas and original corpus closure. No bulk moved. Old executable bindings checked; prescribed warm/corpus executables preserved. Deletion ledger and replacement records: `run24-cleanup.json`; compression deferred per brief Update 3 (.reference below 10 GiB allocated).

Disk: checkout **18.56 / 30.85 GiB allocated / apparent**, `.reference` **6.90 / 18.05 GiB**, `target` **9.67 / 10.64 GiB**; free **111.77 GiB**. All ceilings pass. Hidden paths/.git and every apparent hard-linked path included. Net checkout growth: **8.72 / 7.72 MiB allocated / apparent**, distinct from gross cleanup savings. Audit: `run24-disk.json` and raw du/df files.

### Run 23 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the entire brief through **Update 10**.
Fixed the next PPTX package cluster: `how_to_solve_problems_pptx`,
`pptx-theme-overrides`, `no-nav-pptx`, and the separate `export_ppt`
extension diagnostic. No commits; inherited changes and untracked `29` preserved.

Static comparison with `lib/pptx/pptx.go:203–233,322–350` found the native
writer copied template files before content types and dropped directory
members. It now writes `[Content_Types].xml` first, raw-copies the complete
template inventory with compressed bytes/metadata, skips the same two
relationship parts and regenerates them (including
`templates/slidemaster_rels.xml`). Regression coverage checks first entry,
unique names, complete template bytes/compression and generated-member count.
The CLI rejects `.ppt` before reading source or choosing stdout format, as in
`d2cli/main.go:246–248`; a regression checks this precedence.

| scoreboard tier | before | run 23 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 51/70 | **55/70** | **0** |
| CLI complete artifacts | 39/70 | **40/70** | **0** |
| CLI retained goldens | 30/61 | **30/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor text+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

Fresh Go passes all 69 enabled CLI cases. CLI/e2e/language are fresh runs;
sketch/exporter rescore run-22 captures with their unchanged hash-verified
adapter (neither changed path executes there). Exporter assertions remain
**2/2**. The initial CLI capture missed an imported-file watch change while
native tests ran concurrently; its raw capture and negative delta remain.
The same binary passes all previous watch assertions in the final isolated
capture. No watch code, comparator or exclusion changed.

`stdout_pptx` now passes package validation and exposes the next failure:
`docProps/core.xml` uses a hard-coded title instead of the input basename
(`d2cli/main.go:630–644`). Three newly passing PPTX cases still fail strict
artifact bytes. Those differences and PDF artifact differences remain
non-passing. Full D2/API parity is incomplete.

**205 CLI + 202 export + 33 harness tests pass**; the CLI shared suite also
passes 205 tests through its example target. Diagnostic build, changed-file
rustfmt and `git diff --check` pass. Raster harness unchanged, not rerun.
Evidence: `.reference/codex-runs/run23-{cli-final,e2e-final,language,sketch,exporter}.json`,
five final deltas, tests/build logs, `run23-validation.json` and `report-run23.md`.
CLI captures use 1 GiB total / 128 MiB file / 20 GiB minimum-free guards;
no timeout or budget exhaustion. Final capture is 5,101,952 bytes, largest
file 722,768 bytes. Latest aliases updated atomically; negative evidence retained.

Next: PPTX document title/metadata, then four watch multiboard output-inventory
mismatches. Remaining assertion failures are listed in the run-23 report;
external layout-plugin behavior remains excluded.

Cleanup retired **4,158 idle cache/temporary files**, recovering **368.37 MiB allocated / 354.27 MiB apparent**. Both Cargo profile locks held and scoped lsof clear; no corpus artifacts/current executables retired or bulk moved. Ledger: `run23-cleanup.json`. Older executable references checked (`run23-old-binary-bindings.txt`); retained warm/corpus bindings preserved. Original input/output hashes verify as recorded in `run23-validation.json`. Compression deferred per brief Update 3 (.reference below 10 GiB allocated).

Disk: checkout **18.55 / 30.84 GiB allocated / apparent**, `.reference` **6.89 / 18.04 GiB**, `target` **9.67 / 10.64 GiB**; free **112.15 GiB**. All ceilings pass. Hidden paths/.git and every apparent hard-linked path included. Net checkout growth: **274.91 / 270.73 MiB allocated / apparent**, distinct from gross cleanup savings. Audit: `run23-disk.json` and raw du/df files.

### Run 22 continuation (2026-10-23; previous checkpoint)

Read newest reports/messages, STATUS and the full brief through **Update 10**.
Fixed the queued shorthand-target cluster: `target-b`, `target-invalid`,
`target-nested-index`, `target-nested-index2`, plus `watch-underscore-link`.
No commits; inherited changes and untracked `29` preserved.

The shared native board resolver now follows `d2target/d2target.go:108–169`
(`Diagram.GetBoard`): optional wrapper components, layers/scenarios/steps
search order and a terminal current-board name. Ordinary object maps remain
ineligible. Missing-target diagnostics join parsed components as in
`d2cli/main.go:538–540`. The CLI process adapter also stopped adding a second
program-name prefix: pinned util-go `xmain/xmaintest.go:202` already adds it.
An end-to-end adapter regression preserves the complete multiline diagnostic.
The initial run-22 capture retains that adapter failure; its separate delta
adds only `target-invalid`. No comparator or exclusion changed.

| scoreboard tier | before | run 22 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 46/70 | **51/70** | **0** |
| CLI complete artifacts | 35/70 | **39/70** | **0** |
| CLI retained goldens | 27/61 | **30/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor text+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

All scoreboards use fresh runs and rebuilt native runners. Fresh Go passes
all 69 enabled CLI cases; exporter pre-reset assertions remain **2/2**.
`watch-underscore-link` now passes navigation, but joins the three earlier
watch cases with a differing multiboard output inventory. Its artifact tier
remains non-passing. Strict PDF/PPTX bytes and other artifact differences
also remain non-passing; full D2/API parity is incomplete.

**204 CLI tests, seven board tests, 33 harness tests and the Go adapter test
pass.** Diagnostic/oracle builds, changed-file rustfmt and `git diff --check`
pass. Raster harness unchanged, not rerun. Evidence:
`.reference/codex-runs/run22-{cli-final,e2e-final,language,sketch,exporter}.json`,
five scoreboard deltas, the separate adapter delta, tests/build logs,
`run22-validation.json` and `report-run22.md`. Fresh CLI/tier captures use
1 GiB total / 128 MiB file / 20 GiB minimum-free guards, without exhaustion
or timeout. Latest aliases updated atomically; negative baselines retained.

Next: the PPTX structure/assertion failures, then four watch multiboard
output-inventory mismatches. Remaining CLI failures are inventoried in the
run-22 report; external layout-plugin behavior remains excluded.

Cleanup retired **4,575 idle Go action-cache/Cargo codegen files**, recovering
**800.26 MiB allocated / 754.98 MiB apparent**. Both Cargo profile locks held
and lsof clear; no corpus artifacts/current executables retired or bulk moved.
Ledger: `run22-cleanup.json`. Original hashes verify 328 e2e inputs, 1,878
expected outputs, 930 bound language expectations, 53 historical unbound
expectations, 61 CLI and 58 sketch/exporter goldens. Compression deferred
per brief Update 3 (.reference below 10 GiB allocated).

Disk: checkout **18.29 / 30.58 GiB allocated / apparent**, `.reference` **6.87 / 18.03 GiB**, `target` **9.42 / 10.38 GiB**; free **112.53 GiB**. All ceilings pass. Hidden paths/.git and every apparent hard-linked path included. Net checkout growth: **34.23 / 30.08 MiB allocated / apparent**, distinct from gross cleanup savings. Audit: `run22-disk.json` and raw du/df files.

### Run 21 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the complete brief through
**Update 10**. Targeted the five queued watch failures. Four now pass the
original upstream assertions: `watch-regular`, `watch-ok-link`,
`watch-nested-layer-link`, and `watch-imported-file`. No commits; inherited
changes and untracked `29` preserved.

Static source comparison found Rust bound `localhost` to the resolver's first
successful address (`::1` here), while Go's `net.Listen` prefers IPv4
(`d2cli/watch.go:465–472`, pinned Go 1.27.0 `net/dial.go:889`). The native
listener now follows that selection and preserves explicit IPv6. Broadcast
counts and sorted changed-file notices now follow `watch.go:296–309,619–637`.
The original import test verifies changes to main and imported files, their
rendered contents, and removal of the dependency watch after deleting its import.

| scoreboard tier | before | run 21 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 42/70 | **46/70** | **0** |
| CLI complete artifacts | 34/70 | **35/70** | **0** |
| CLI retained goldens | 27/61 | **27/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor text+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

`watch-imported-file` also passes the complete artifact contract. Three other
watch cases still differ in output inventory: Go writes the multiboard tree;
Rust writes a single `index.svg`. They remain artifact mismatches. The fifth
case, `watch-underscore-link`, reaches navigation but fails when selecting
`cream` without the `layers` wrapper. No comparator or exclusion changed.
Fresh Go passes all 69 enabled cases. CLI/e2e/language results are fresh;
sketch/exporter rescore retained captures with the unchanged hash-verified
adapter, whose paths do not execute the watch server. Exporter pre-reset
assertions remain **2/2**. Full D2/API parity is incomplete.

**204 native CLI tests + 33 scoreboard harness tests pass.** The final
changed-file notice adjustment additionally passes all **15 watch tests** and
the fresh upstream CLI capture. Diagnostic builds, changed-file rustfmt and
`git diff --check` pass. Raster harness unchanged, not rerun.

Evidence: `.reference/codex-runs/run21-{cli-final,e2e-final,language,sketch,exporter}.json`,
five delta files, `run21-validation.json`, build/test logs and `report-run21.md`.
Raw final CLI capture: `.reference/cli-scoreboard-20261023/run21-final/`
(**5,117,414 bytes**, largest **722,768 bytes**). The earlier `run21/` capture
and its negative report retain the import-log failure. Both runs use the
1 GiB total / 128 MiB file / 20 GiB minimum free guard, without timeout or
budget exhaustion. Latest scoreboard aliases replaced atomically.

Next by combined impact: the four shorthand-target failures (`target-b`,
`target-invalid`, `target-nested-index`, `target-nested-index2`) plus
`watch-underscore-link`. Compare `d2target.go:108` GetBoard/name traversal with
`compiler/boards.rs:67`'s alternating wrapper/name resolver and CLI target
plumbing. Then four PPTX ZIP-first-entry failures and three watch multiboard
output-inventory mismatches. Strict PDF bytes and other artifact differences
remain non-passing.

Cleanup removed **368.37 MiB allocated / 354.26 MiB apparent** from the idle
Go action cache after lsof found no open files (`run21-cleanup.json`). No
corpus artifacts retired or bulk moved. Original hashes verify 328 e2e
inputs, 1,878 expected outputs, 930 bound language expectations, 53 historical
unbound expectations, 61 CLI goldens and all 58 sketch/exporter goldens.
Compression deferred per brief Update 3 (.reference below 10 GiB allocated).
Disk: checkout **18.26 / 30.55 GiB allocated / apparent**, `.reference` **6.85 / 18.01 GiB**, `target` **9.40 / 10.37 GiB**; free **114.06 GiB**. All ceilings pass. Net checkout growth: **13.27 / 11.46 MiB allocated / apparent**, distinct from gross cleanup savings. Audit: `run21-disk.json` and raw du/df files.

### Run 20 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the full brief through **Update
10**. Fixed the largest measured CLI export cluster: `internal_linked_pdf`,
`stdout_pdf`, `theme-pdf`, `no-nav-pdf`, and `renamed-board` now pass the
original pinned Go assertions. No commits; inherited changes preserved.

Rust wrote uncompressed RGB/alpha image streams. Original
`lib/pdf/pdf.go:86` registers PNGs with FPDF; pinned fpdf v0.12.0
`png.go:85,170–246` and `fpdf.go:4791–4823` publish FlateDecode streams with
PNG predictor 15 and a grayscale soft mask. The native writer now compresses
decoded rows with PNG filter 0 and declares the correct channel count and
columns for each plane. Both PDF package paths and CLI navigation use the
shared writer; compression failures propagate. No raster algorithm changed.

| scoreboard tier | before | run 20 | regressions |
|---|---:|---:|---:|
| CLI original assertions | 37/70 | **42/70** | **0** |
| CLI complete artifacts | 34/70 | **34/70** | **0** |
| CLI retained goldens | 27/61 | **27/61** | **0** |
| e2e SVG / expected errors | 933 / 11 | **933 / 11** | **0** |
| language JSON / editor text+errors | 915 / 445 | **915 / 445** | **0** |
| sketch / exporter goldens | 38/39 / 19/19 | **38/39 / 19/19** | **0** |

All five PDFs remain strict artifact-byte mismatches; they are not claimed as
complete artifact or golden matches. The CLI delta now independently guards
the original assertions even when artifact bytes differ, with a regression
test for that distinction and capture-failure/unsupported gates. Existing
comparators and exclusions are unchanged. Go independently passes all 69
enabled cases. Fresh native CLI capture, fresh e2e and language runs pass
their regression guards; sketch/exporter rescored retained captures using the
unchanged, hash-verified adapter executable (PDF writing is outside those
tiers). Exporter pre-reset assertions remain **2/2**.

**202 native exporter tests + 33 scoreboard harness tests pass.** The PDF
regression decodes actual RGB and soft-mask streams and checks their lengths,
row boundaries and pixels. All **16 planes in the five failing-case PDFs**
also decode to exactly the prior native pixels/dimensions/channels. Their
combined size fell from **25,209,795 to 193,856 bytes**. Native diagnostic
build, changed-file rustfmt and `git diff --check` pass. Raster harness is
unchanged and was not rerun.

Evidence: `.reference/codex-runs/run20-{cli,e2e-final,language,sketch,exporter}.json`,
five delta files, `run20-pdf-pixels.json`, `run20-validation.json`, build/test
logs and `report-run20.md`. Fresh raw CLI capture:
`.reference/cli-scoreboard-20261023/run20/` (**5,119,744 bytes**, largest file
**722,768 bytes**); 1 GiB total / 128 MiB per file / 20 GiB minimum free
guard, no exhaustion or timeout. Latest scoreboard aliases updated atomically;
all previous negative baselines remain.

Next: five watch failures, then four PPTX ZIP-structure failures and four
shorthand-target failures. Confirm the common source cause before changing
each cluster. Preserve all scoreboards; strict PDF bytes and other artifact
differences remain non-passing. Full D2/API parity remains incomplete.

Cleanup removed **1.09 GiB allocated** of idle Go action cache and completed
compiler intermediates (`run20-cleanup.json`). No corpus artifacts retired or
bulk moved. After cleanup, original hashes verify **328 e2e inputs / 1,878
expected outputs / 930 bound language expectations / 53 historical unbound
expectations / 61 CLI goldens / 58 sketch+exporter goldens**. Compression
deferred under brief Update 3 (.reference below 10 GiB allocated).

Disk: checkout **18.25 / 30.54 GiB allocated / apparent**, `.reference` **6.84 / 18.00 GiB**, `target` **9.40 / 10.37 GiB**; free **113.30 GiB**. All ceilings pass. Net checkout change: **-282.93 / -273.36 MiB allocated / apparent**, distinct from gross cleanup savings. Audit: `run20-disk.json` and raw du/df files.

### Run 19 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the entire brief through **Update
10**. Added the two remaining golden-tier scoreboards before selecting more
native fixes. No production Rust implementation changed; no commits made.

`scripts/golden-tier-scoreboard.py` captures the original pinned Go test
bodies through `tools/golden-tier-scoreboard/hook_test.go` in the trace checkout.
The native example `crates/d2-rs-cli/examples/golden-tier-scoreboard.rs` reuses
the CLI's native compile/measure/layout/export path.

| new tier | first qualified result | remaining non-passing |
|---|---:|---|
| d2sketch complete SVG goldens | **38 / 39** | historical `dots` has no current test input |
| d2exporter complete JSON golden contracts | **19 / 19** | none |
| d2exporter original pre-reset assertions | **2 / 2** | none |

Go independently reproduces all 57 bound goldens and passes every current
case. Sketch uses only the established e2e XML/version/newline allowance.
Exporter resets exactly the geometry fields in upstream `export_test.go:284–297`;
root, legend and arrowhead metadata remain intact. JSON uses the language
scoreboard's type-aware comparator. The initial harness omitted empty connection
`labelPosition` fields, producing ten false mismatches. `d2target.go:812,918`
shows that only shape label positions are omitempty. Corrected the harness, added
a regression test and reran fresh captures: **+10 harness matches / 0 regressions**,
not ten native fixes. The affected cases are named in `run19-exporter-delta.txt`.

Fresh e2e start/end: **933 SVG + 11 expected errors**, delta **0 / 0 regressions**.
Fresh language: **915 complete JSON + 445 editor text/error matches**, zero
regressions. Rescored retained CLI captures with the unchanged executable:
**34/70 contracts + 27/61 goldens**, zero regressions. The new sketch tier's
development delta is zero. All **32 scoreboard harness + 33 raster harness
tests pass**; the native adapter builds with the oracle profile. Production
Rust suites were not rerun because production code is unchanged.

The shared capture guard now also counts Go's temporary `.got` files beside
the goldens and tolerates their removal during scanning. A sparse-file test
verifies that these files can exhaust the budget. Actual captures stayed
within 1 GiB total / 128 MiB per file / 20 GiB minimum free: **7,024,913 bytes**
retained, largest file **474,023 bytes**, no capture exhaustion or timeout.
Raw inputs, targets, SVGs, logs and hash inventories are retained at
`.reference/golden-tier-scoreboard-20261023/{sketch-final,exporter-final}/`.
`--reuse` verifies the executable and every captured file before rescoring.

Evidence: `.reference/codex-runs/run19-*.json`, the five delta files,
`run19-validation.json` and `report-run19.md`. Latest aliases replaced
atomically. After cleanup, original hashes verify **328 e2e inputs / 1,878
expected outputs / 930 bound language expectations / 53 historical unbound
expectations / 61 CLI goldens / all 58 new-tier goldens**, plus every new capture.

The next native queue is the CLI scoreboard: five PDF image filter/predictor
failures, five watch address failures, four PPTX structure failures and four
shorthand-target failures (`target-b`, `target-invalid`, `target-nested-index`,
`target-nested-index2`). These are symptom clusters; confirm a common source
cause before changing code. Preserve all five scoreboards. `dots` remains
unbound, and external layout-plugin discovery/protocol remains excluded.

Disk: checkout **18.52 / 30.81 GiB allocated / apparent**, `.reference`
**7.39 / 18.53 GiB**, `target` **9.14 / 10.11 GiB**; free
**111.77 GiB**. All ceilings pass. Net retained checkout growth is
**67.98 / 66.62 MiB allocated / apparent**.
Audit and deletion ledger: `run19-disk.json`, `run19-cleanup.json`.
Deleted **476.71 MiB allocated** of idle compiler intermediates and superseded
development captures after verifying identical retained raw witnesses. No
corpus inputs, expected outputs or current binaries removed; historical paths
have explicit replacements. Compression deferred per brief Update 3
(`.reference` below 10 GiB allocated). Full D2/API parity remains incomplete.

### Run 18 continuation (2026-10-23)

Read the newest reports/messages, STATUS and the full brief through **Update
10**. The completed e2e corpus is now a regression guard, not the work queue.
No Rust implementation changed and no commits were made.

Added `scripts/cli-scoreboard.py`, its delta checker and the test-only native
process adapter in `tools/cli-scoreboard/hook_test.go`. It inventories all
**64 main CLI cases, six current PNG/GIF fixture cases and 61 retained golden
files**. Original Go test bodies and assertions are retained. Every enabled
Go case passes (**69 pass, one upstream skip**). A historical golden passes
only when Go reproduces its actual artifact, independently of current assertions.

| CLI tier | first qualified baseline | remaining non-passing |
|---|---:|---|
| complete case contracts and output inventories | **34 / 70** | 30 Rust assertion failures, 3 artifact differences, 2 unsupported injected Go writers, 1 upstream skip |
| retained golden artifacts | **27 / 61** | 19 Rust failures, 11 stale/unreproduced Go artifacts, 4 historical unbound files |

The 46 Go-reproduced artifacts include all six current raster fixture goldens.
PNG uses `collect_png_outputs` / `compare_png_outputs`; GIF compares all decoded
RGBA frames, timing, canvas and loops. SVG uses only the existing e2e
XML/version/newline allowance; the upstream test library's version is
`v0.8.1-HEAD`. Raw captures remain unchanged. Empty declared icon inputs are
recorded separately, never accepted as empty rendered output. PDF/PPTX byte
comparison is conservative and does not yet distinguish incidental encoding
differences. Go writer injection cannot be represented by this process adapter.

The guard covers active temporary outputs and logs (1 GiB total, 128 MiB per
file, at least 20 GiB free); no budget exhaustion or missing test event occurred.
Capture manifest binds pinned source, native executable and hook hashes.
Source plumbing follows `e2etests-cli/main_test.go:1744–1767,1784–1809` and
the process-cwd font read in `d2cli/main.go:1109`. No fixtures were excluded.

Fresh e2e start/end: **933 SVG + 11 expected errors**, **0 regressions**.
Fresh language: **915 complete JSON + 445 editor source/error matches**,
**0 regressions**; all 53 historical unbound rows remain non-passing.
The CLI development delta is +1 / zero regressions from correctly identifying
nested empty input icons; it is a harness correction, not a native parity fix.
**9 CLI harness tests and all 33 raster harness tests pass**. Rust suites were
not rebuilt: native code and the scored executable are unchanged.

Evidence: `.reference/codex-runs/run18-{e2e-start,e2e-final,language,cli-final}.json`,
the three `run18-*-delta.txt` files, and `report-run18.md`. Current raw captures
and reproduction manifest: `.reference/cli-scoreboard-20261023/final/`.
Next: add d2sketch (39) and d2exporter (19) scoreboards, then prioritize the
largest combined failure cluster. CLI symptoms include five PDF inspector
failures, four PPTX structure failures, five watch address failures, multiboard
links/targets, custom fonts and diagnostics. Two GIFs differ in decoded
playback; successful validation emits extra stdout. No shared cause is claimed.
`layout-extra-args` remains inventoried, but its external layout-command failure
is not a completion gate under the project's plugin exclusion.

Disk: checkout **18.46 / 30.74 GiB allocated / apparent**, `.reference`
**7.37 / 18.51 GiB**, `target` **9.09 / 10.06 GiB**, free **112.15 GiB**.
All ceilings pass. Deleted **197.95 MiB allocated** of superseded development
captures, temporary assertion diffs and the idle score executable clone;
current captures, pinned corpus inputs/expected outputs and native deliverables
remain. Gross savings differ from net checkout growth (**81.36 MiB allocated /
75.12 MiB apparent**). Ledger/audit: `run18-cleanup.json`, `run18-disk.json`.
Post-cleanup hashes verify 328 e2e inputs, 1,878 expected outputs, 930 bound
language expectations, 53 historical unbound expectations and all 61 CLI
goldens; both engines retain all 69 enabled case captures. No bulk moved out
of the checkout. Compression deferred per brief Update 3 (.reference <10 GiB).

### Claude continuation after the Codex handover (2026-10-23; completed e2e)

Codex stopped at 90% quota after run 17. Its 912 was independently re-scored
from a fresh build before continuing (identical, zero delta).

**912 → 933 exact SVGs, zero regressions** (full e2e scoreboard, delta against
the re-scored run-17 baseline). **Every applicable upstream e2e board now
matches** (933/933 svg-match; the other 11 applicable boards are error-matches). ELK target mismatches 10 → **0**: every
applicable ELK e2e board is byte-identical. All fixes are in
`crates/d2-rs-elk` and were found by running Go D2's own ELK wire input
(`D2_ELK_WIRE_TRACE` in `../d2-v0.9.0-trace`) through pinned elk-go and the Rust
port, then diffing per-processor layer contents with a scratch-patched elk-go:

| fix | elk-go / D2 source | boards |
|---|---|---|
| FIRST/LAST layer constraints for nodes left edgeless by hidden external ports, re-placed after layering | `layer_constraint_preprocessor.go:107–130`, `layer_constraint_postprocessor.go:14–131` | +5: `real_world/{mocha_soc,ross_overview,tpmjs_architecture}`, `regression/elk_order`, `stable/large_arch` |
| self-loop label side selection, side-only slot widening, inline West/East labels and margins, opposing-loop activity, D2 self-loop label deconfliction | `loops/routing/{label_placer,orthogonal_router,slot_assigner}.go`, `d2elklayout/layout.go:673–740` | +1: `stable/ent2d2_right` |
| removed non-source post-BK fan-in/fan-out alignment heuristics | none in elk-go (BK result is final) | +3: `real_world/queue_workers`, `regression/straight_hierarchy_container_direction_right`, `txtar/theme-overrides` |
| BK in-layer separation counted label overflow twice (margin already includes it) | `p4nodes/bk/compactor.go` | +1: `stable/icon_positions` |

Witnesses (pinned-Go byte-identical SVG, CLI regression assertions):
`tests/regressions/e2e/elk-{first-constraint-order,opposing-loop-labels,external-fanin-order,label-overflow-spacing}.d2`.
Tests: d2-rs-elk 236, d2-rs-cli 203 (both targets), d2-rs-layout 100 pass.
`remote_asset_timeout_covers_headers_and_body` (30 ms localhost timeout) is
flaky under parallel load only; it passes alone and with `--test-threads=4`.

Dagre (3 → 0 target mismatches, zero regressions; details in
`crates/d2-rs-dagre/SOURCE.md`): outside-label step-back ported from
Edge.TraceToShape; the curve clip now also traces non-rectangular endpoint
shapes before the target intersection, as `d2graph/layout.go` TraceToShape does
(+1 `stable/chaos2`); the non-source `reorder_mixed_external_anchor_positions`
/ `shift_reordered_external_routes` / `canonical_endpoint_node` heuristic was
removed (+1 `todo/container_label_edge_adjustment`); and fitted compound
origins are truncated at publication like d2exporter's `int()` while all other
Dagro boxes keep layout.go:202's rounding (+1 `regression/dagre_broken_arrowhead`).
Witnesses: `tests/regressions/e2e/dagre-{fitted-origin-truncation,mixed-external-anchor-order}.d2`.
Four non-source Dagre route overrides were measured as inert on the corpus
(922 with and without them); they are candidates for removal.

TALA (after the checkpoint commit): **922 → 930 exact SVGs, zero regressions**.
Isolated-tree eligibility now checks every root of the sentinel
(`trees/preprocess_helpers.go:623–648`; +1 `txtar/link-on-connections`), and the
compound candidate plus existing-label quality scorer read live node positions
instead of stale arena copies (+3 `stable/{dagre_spacing_right,elk_shim,investigate}`).
Method: TALA_TRACE_JSONL stage snapshots from `../d2-v0.9.0-trace` against the
Rust stage trace (all 48 `elk_shim` stages equal pinned Go), plus instrumented
candidate/quality scores. Witnesses: `tests/regressions/e2e/tala-{isolated-tree-sibling-label,compound-live-label-score}.d2`.
Then NodePlacement: the committed sized swap now publishes workspace positions to
the owner before `SyncHerdFences` (+1 `txtar/sql-icon`, witness
`tala-herd-fence-after-swap.d2`), and mirror reachability expands cluster/sequence
vessels into their members as `reachableNodesGuarded` does (+1 `stable/large_arch`),
and horizontal hierarchies are transposed around placement and mirrored for
left/up as `placeNodesInHierarchy` does (+1
`regression/straight_hierarchy_container_direction_right`, witness
`tala-horizontal-hierarchy-transpose.d2`).
TALA 1681, CLI 203 tests pass.

Then the common-uncle map groups active cluster vessels through their members'
edges as `proximity/common_uncle.go` does (+1 `real_world/ross_overview`).
Tests: d2-tala-rs 1681, d2-rs-cli 203 (both targets), d2-rs-dagre 108,
d2-rs-layout 100 pass.

Remaining e2e target mismatches: **0**. The scoreboard now only guards
regressions; remaining parity work is outside the upstream e2e corpus.

### Run 17 continuation (2026-10-23; final Codex handover)

Read newest reports/messages, STATUS and the entire brief through **Update 9**.
Following its final-run scope, fixed one small source-mapped cluster and
stopped parity development. Inherited changes preserved; no commits.

**911 → 912 exact SVGs, zero regressions.** Dagre
`txtar/bidirectional-connection-animation` now matches the complete golden;
a fresh pinned-Go CLI run matches independently. Original
`d2layouts/d2dagrelayout/layout.go:202–204,235–240` publishes rounded object
boxes before adding their centers as self-loop endpoint sentinels, while
Dagro's interior controls keep fractional coordinates. Rust used the model
center for both. `dagre_self_loop_points` now preserves the source distinction.

| e2e outcome | run 17 start | qualified |
|---|---:|---:|
| exact SVG | 911 | **912** |
| expected error | 11 | **11** |
| target mismatch | 22 | **21** |
| runtime failure | 0 | **0** |

All 966 rows and every prior match remain; no comparison normalization,
fixture-specific implementation or exclusion changed. Two-line reduced
witness: `tests/regressions/e2e/dagre-self-loop-published-center.d2`, target
3482141975 → **1485139401**. Complete normalized Go/Rust SVG equality includes
hash-dependent IDs. Added to the CLI's pinned-Go regression suite.
Evidence: `.reference/codex-runs/run17-{start,after,fixed,witness}.json`,
`run17-e2e-delta.txt`, build/test logs and `report-run17.md`.

**310 distinct native tests pass** (203 CLI + 107 Dagre), plus the CLI's
203 duplicate existing-target invocations; zero failures/ignores and one
empty doctest suite. Command: `cargo test --profile oracle -p d2-rs-dagre
-p d2-rs-cli -- --test-threads=1`. Two preceding parallel runs each hit the
inherited 30 ms localhost timeout assertion (one in each CLI target); both
logs are retained. No timeout/test implementation changed. Changed-file
rustfmt and `git diff --check` pass.

Next: **21 target mismatches** (three Dagre / 10 ELK / eight TALA).
`report-run17.md` groups every board by root-cause hypothesis and gives next
witness/diff commands; `run17-remaining.json` is the exact current inventory.
ELK ordering/hierarchy geometry is the largest suspected group. Rebuild
current diagnostic traces before further investigation; the shared trace
alias remains run 16's baseline. Language/WASM/raster were not rerun and
exports still require a concrete failing case. Full D2/API parity is incomplete.

### Run 17 disk audit (2026-10-23)

All ceilings pass: checkout **16.95/29.27 GiB allocated/apparent**, `.reference` **6.77/17.93 GiB**, `target` **8.17/9.16 GiB**; free **77.44 GiB**.
Cleanup deleted **462 files / 849.36 MiB allocated** (idle codegen, redundant run executable snapshot and completed scratch). Net checkout change after builds: **+1.16 MiB allocated / +0.87 MiB apparent**.
Hidden paths/.git and all apparent hard-linked paths included. No bulk moved.
Ledger/audit: `run17-cleanup.json`, `run17-retirement.json`, `run17-disk.json`
and `run17-disk-final-*.txt`. Both Cargo locks held and lsof clear during
cleanup; current warm executables retained. After cleanup, 328 e2e inputs,
1,878 expected outputs, 930 bound language golden hashes and 53 historical
unbound hashes verify (`run17-closure-validation.json`). No corpus input,
expected output or user deliverable removed. Compression deferred per brief
Update 3 (.reference below 10 GiB allocated).

### Run 16 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the entire brief through
**Update 8**. Fresh baseline exactly reproduces run 15. Rebuilt the current
diagnostic binary and diffed all 25 remaining boards; preserved inherited
changes and made no commits. No language, WASM or export rewrite.

**908 → 911 exact SVGs, zero regressions.** Three source-mapped fixes:

- Dagre `txtar/md-tables`: outside label/icon clipping now uses the same
  temporarily shifted 3D/multiple box as shape clipping. Original
  `d2layouts/d2dagrelayout/layout.go:324–343` moves the box before
  `d2graph/layout.go:414–438` computes the outside label rectangle.
- Dagre `stable/grid_icon`: an empty detached grid root discards the
  provisional parent-engine label position. Original
  `d2layouts/d2grid/layout.go:31–35` leaves that label position nil;
  `d2layouts/d2layouts.go:748–753` injects the root state after parent layout.
- ELK `txtar/sql-table-reserved`: preserve the solver's SQL column endpoint
  after outside margins are removed. The wire pipeline no longer overwrites
  that endpoint from the shrunken table; this created a diagonal final segment.
  Original `d2layouts/d2elklayout/layout.go:366–389,541–606,608–635` creates
  ports on the inflated wire box, removes margins and then traces the route.
  The separate legacy routing APIs keep their existing port helper.

| e2e outcome | run 16 start | qualified |
|---|---:|---:|
| exact SVG | 908 | **911** |
| expected error | 11 | **11** |
| target mismatch | 25 | **22** |
| runtime failure | 0 | **0** |

All 966 rows and every previous match remain. Each fix's full scoreboard delta
is +1 / zero regressions; the final delta is +3 / zero regressions. No golden
exclusions, normalization changes or fixture-specific implementation paths.
Reduced witnesses in `tests/regressions/e2e/`:
`dagre-modifier-label-trace.d2` (3890689203 → **2721557747**),
`grid-empty-label-icon.d2` (3546686747 → **2599487348**) and
`elk-sql-icon-port.d2` (759833929 → **1787285706**). Each matches complete
normalized pinned-Go SVG bytes and is in the CLI regression suite.

Evidence: `.reference/codex-runs/run16-{start,cluster,after-modifier,
after-grid,after,witness,fixed,remaining}.json`, per-fix delta text,
`run16-e2e-delta.txt` and `report-run16.md`. The short-endpoint repair
experiment fixed no scoreboard boards and was removed; its zero-delta
report is retained. **646 distinct native tests pass** (203 CLI, 107 Dagre,
236 ELK, 100 layout), plus the CLI's 203 duplicate existing test-target
invocations. No failures or ignored tests in these selected suites; three
empty doctest suites complete successfully. Changed-file rustfmt and
`git diff --check` pass. Final disk results are recorded below.

Next: **22 target mismatches** (four Dagre / 10 ELK / eight TALA), with
ELK ordering/hierarchy geometry the largest unresolved group. Current
remaining inventory is `run16-remaining.json`; `run16-cluster.json` is the
pre-fix field diff and the shared trace binary is the run-16 baseline.
Native exports still need a concrete failing case before changes. Language,
WASM and raster gates were not rerun, and full D2/API parity is incomplete.

### Run 16 disk audit (2026-10-23)

All ceilings pass: checkout **16.95/29.27 GiB allocated/apparent**, `.reference` **6.77/17.93 GiB**, `target` **8.17/9.16 GiB**; free **78.20 GiB**.
Cleanup recovered **1,764.71 MiB allocated / 935 files**: completed Cargo codegen, five superseded test executables, disposable run-16 executable snapshots and scratch captures. Net checkout change after builds: **-113.47 MiB allocated / -208.27 MiB apparent**.
Hidden directories/.git and every apparent hard-linked path are included.
Ledgers: `run16-cleanup.json`, `run16-retirement.json`; measurements:
`run16-disk.json` and `run16-disk-final-*.txt`. Both Cargo locks held and
lsof clear during deletion; current warm executables remain. All 328 e2e
inputs, 1,878 expected outputs, 930 bound language golden hashes and 53
historical unbound hashes verify after cleanup (`run16-closure-validation.json`).
No corpus input, expected output or user deliverable removed; no bulk moved.
Compression deferred per brief Update 3 (.reference below 10 GiB allocated).

### Run 15 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the entire brief through
**Update 8**. Fresh e2e baseline reproduces run 14. Pinned Go independently
matches all 28 remaining goldens; no exporter rewrite or language work was
repeated. Prior edits remain preserved; no commits.

**TALA compound-result publication fixed: 905 → 908 exact SVGs, zero
regressions.** `stable/all_shapes_link`, `stable/direction` and
`stable/nesting_power` now match complete upstream SVGs. After accepting a
rigid-container proposal, `refresh_seed_layout_result` read the arena
record's stale position instead of the live owner point. It now uses
`graph.position(node.input_id)`, matching the ordinary snapshot path and
pinned `d2layouts/d2talalayout/patch.go:330–347` (`buildObjectPatch` copies
the selected node's live `TopLeft`). The proposal is rerouted/normalized by
`internal/engine/compound_candidate.go:113–148`. Complete Go/Rust
`SelectedResult` events for `stable/direction` were already identical:
the earliest divergence was result publication, not layout selection.

| e2e outcome | run 15 start | qualified |
|---|---:|---:|
| exact SVG | 905 | **908** |
| expected error | 11 | **11** |
| target mismatch | 28 | **25** |
| runtime failure | 0 | **0** |

All 966 rows and every previous match remain. No comparison normalization,
golden exclusion or layout heuristic changed. Six-line reduced witness:
`tests/regressions/e2e/tala-compound-live-position.d2`; before target
2694434806, fixed/pinned-Go target **2101581790**, with complete normalized
SVG equality. Added to the CLI's pinned-Go target regression suite.

Evidence: `.reference/codex-runs/run15-{start,after,go-remaining,cluster,
publication-witness,witness,fixed,remaining}.json`, `run15-e2e-delta.txt`
and `report-run15.md`. **1,884 distinct native unit tests and two doctests
pass** (203 CLI + 1,681 TALA); the CLI's second existing Cargo target also
passes all 203.
The 24 existing TALA ignores remain non-passing, unchanged. Changed-file
rustfmt and `git diff --check` pass. Final disk results are recorded below.
Language/WASM/export scoreboards were not rerun; no new claims for those
tiers. Shared TALA result publication also benefits native API consumers.

Next: **25 target mismatches** (six Dagre / 11 ELK / eight TALA), largest
root-cause cluster first. `run15-cluster.json` contains the pre-fix field
diffs; `run15-remaining.json` is the current board inventory. Rebuild the
diagnostic trace binary before further investigation. Full D2/API parity
remains incomplete; export changes still require a concrete failing case.

### Run 15 disk audit (2026-10-23)

All ceilings pass: checkout **17.06/29.48 GiB allocated/apparent**, `.reference` **6.76/17.93 GiB**, `target` **8.28/9.36 GiB**; free **78.20 GiB**.
Cleanup recovered **2,022.17 MiB allocated / 3,560 files** (idle Go action cache, completed Cargo codegen files, disposable run executables/captures and two superseded CLI test executables). Net checkout change after builds: **+780.73 MiB allocated / +779.77 MiB apparent**.
Hidden paths/.git and every apparent hard-linked path are included. Ledger/audit:
`run15-cleanup.json`, `run15-disk.json`, `run15-disk-final-*.txt`.
No corpus inputs, expected outputs or user deliverables removed; no bulk moved.
Cargo temporary/executable cleanup held both profile locks; lsof reported no active holders.
Current warm tests remain; historical executable hashes/rebuild commands are in
`run15-retirement.json`. Compression deferred per brief Update 3 (.reference below 10 GiB allocated).

### Run 14 continuation (2026-10-23; previous checkpoint)

Read the newest reports/messages, STATUS and the entire brief through
**Update 8**. That update supersedes run 13's proposed export bridge:
pinned Go uses d2scenebuild/d2raster and the existing raster gate passes
33/33; no exporter rewrite is justified without a failing case. Math,
language bookkeeping, consumed-edge removal and WASM SVG were already done.
Inherited edits remain preserved; no commits.

**TALA Jupyter panic fixed: 904 → 905 exact SVGs, zero regressions.**
`real_world/jupyter_aws_eks` now matches the complete upstream SVG. Native
TALA's grid rerouting visited parent scopes before children, reserving an
unselected child edge label whose position was still unset; the collision
scan then panicked. Use the existing postorder grid traversal for TALA as
well, and place any implicit root last. No panic suppression or obstacle
exclusion was added. Original `d2layouts/d2layouts.go:240–246,315–354`
finishes recursive child layouts before restoring parent crossings;
`d2talalayout/internal/labeling/placement.go:496–547` reserves existing labels.

| e2e outcome | run 14 start | qualified |
|---|---:|---:|
| exact SVG | 904 | **905** |
| expected error | 11 | **11** |
| target mismatch | 28 | **28** |
| runtime failure | 1 | **0** |

The complete 966-row inventory is unchanged. A fresh pinned Go CLI check
also matches the Jupyter golden. Eight-line witness:
`tests/regressions/e2e/tala-nested-grid-routing-order.d2`; the baseline
panics, and the fixed output matches complete normalized Go SVG, target
**1710228742**. The CLI regression suite includes this source-derived hash.
No comparison normalization or golden exclusions changed.

Evidence: `.reference/codex-runs/run14-{start,after-order,jupyter-qualified,
witness}.json`, `run14-e2e-delta.txt`, negative panic logs, test/build logs
and `report-run14.md`. **303 distinct native tests pass** (203 CLI + 100 layout); the CLI suite also
passes under its second existing Cargo test target. Changed-file rustfmt and
`git diff --check` pass. `run14-validation.json` verifies the full inventory,
unchanged passing rows and current aliases. No additional language/WASM/raster
run is claimed; their implementation is unchanged. Final storage audit below.

Next, per Update 8: **28 target mismatches**, largest field cluster first
(six Dagre / 11 ELK / 11 TALA). Native raster/PDF/PPTX changes require a
concrete failing case; the existing 33/33 raster result is prior evidence,
not rerun or migrated here. Language and WASM behavior were not changed.
Full D2/API parity remains incomplete.

### Run 14 disk audit (2026-10-23)

All ceilings pass: checkout **16.30/28.71 GiB allocated/apparent**,
`.reference` **6.76/17.92 GiB**, `target` **7.52/8.60 GiB**;
free **79.35 GiB**. Cleanup recovered **1,074.76 MiB
allocated / 478 files**; net checkout change is
**+78.62 MiB allocated /
+54.49 MiB apparent** after builds.
Hidden directories/.git and every apparent hard-linked path are included.
Deletion ledger: `run14-cleanup.json`; measurements: `run14-disk.json` and
`run14-disk-post-cleanup-*.txt`. No corpus input/output or user deliverable
was removed; no bulk moved. Both Cargo locks held and lsof clear during
cleanup. Brief Update 3 defers compression while .reference is below 10 GiB.

### Run 13 continuation (2026-10-23; previous checkpoint)

Read the full brief through **Update 7**, newest reports/messages, STATUS,
PLAN and scoreboard workflow. Fresh e2e baseline reproduces run 12. Math
notices, UpdateImport and TALA consumed-edge publication were already done.
Inherited edits remain preserved; no commits.

**S6 WASM SVG migration: 0 → 950 valid pinned-Go output matches, zero
regressions.** Supplied targets and source rendering now use d2svg, including
sequence shapes, exact fonts, Markdown, code, LaTeX, sketch, themes, XML,
appendix and animation. Source animation shares the CLI target-tree builder;
the old WASM SVG wrapper reconstruction is removed. Asset-data substitution
preserves target hashes. Source mapping: original
`d2js/d2wasm/functions.go:347–551`; d2svg/appendix/d2animate remain shared Rust.

Both the native boundary runner and the **built release Node WASM module**
reproduce the same full 952-target inventory:

| WASM SVG tier | before | qualified |
|---|---:|---:|
| exact upstream SVG | 0 | **936** |
| exact pinned Go, differing golden/API contract | 0 | **14** |
| rejected by both (non-passing) | 2 | **2** |
| remaining output mismatch/native rendering failure | 950 | **0** |

The 14 consist of the 11 existing stale goldens plus three `sanity/empty`
targets: Go's WASM Render calls renderSingleBoard for these, whereas e2e
calls renderBoards/Wrap. The source-mapped native Go API adapter confirms
all three; no golden was excluded or comparison normalization added. Both
decode rejections remain explicitly non-passing. This is a rendering tier,
not a claim that WASM compilation/layout matches every source board.

Also repaired the inherited post-SetDimensions WASM Graph label projection:
object/edge labels now use measured/transformed native labels, following
`d2graph/d2graph.go:1587–1591,1682–1685`; pre-measurement compiler records stay
raw. The complete 225 public graph object/storage controls pass. Recovered
route faults keep their original message/context transport. A sketch animation
test's total-path equality was false for pinned Go (11 animated paths versus
8 ordinary paths); both complete SVGs match Go bytes. The content inventory
assertion now distinguishes definitions from painted paths.

**567 native tests pass** (100 WASM, 203 CLI, 264 render), plus Node runtime
and TypeScript gates. Connected **Chrome 155** passes all 32 browser contract
groups, including actual module workers and direct fallback. The shell browser
launcher was denied; the same existing runner passed through browser control.
Fresh CLI e2e remains **904 exact SVG / 11 expected errors / 28 target
mismatches / one runtime failure**, **0 regressions**. Language remains
**915 complete JSON / 445 editor source/error matches**, **0 regressions**.

Evidence: `.reference/codex-runs/run13-{start,e2e-qualified,wasm-start,
wasm-qualified,wasm-native,language-qualified}.json`, deltas, test/build logs,
`run13-browser-summary.json`, `run13-animation-witness.json`,
`run13-closure-validation.json`, `run13-validation.json` and `report-run13.md`.
Latest aliases updated atomically; negative baselines remain retained.

**Next: S6 native raster/PDF/PPTX bridge**, then the remaining e2e geometry
and Jupyter runtime failure. Static comparison establishes that pinned Go
uses d2scenebuild/d2raster for these exports (`d2cli/main.go:1005–1032`,
`raster.go`, `paged.go`); simply substituting SVG would discard scene/precision
contracts. The native bridge remains required and has not been retired.
Broader native/WASM/API and full D2 parity remain incomplete.

### Run 13 disk audit (2026-10-23)

All ceilings pass: checkout **16.22/28.66 GiB allocated/apparent**, `.reference` **6.76/17.92 GiB**, `target` **7.45/8.55 GiB**; free **80.87 GiB**.
Hidden paths and every hard-linked apparent path are included. Actual cleanup:
**2,137 files / 288.70 MiB allocated** (idle Go action cache,
two superseded executable copies/dependency records and disposable captures).
Current release WASM package, warm tests, required historical renderer/trace
executables, source and corpus inputs/expected outputs remain. Historical
executable hashes/rebuilds and affected bindings are recorded in
`run13-retirement.json`; no bulk moved or per-run compression pass.

After deletion, all **328 e2e input hashes / 1,878 expected output hashes**,
**930 bound language golden hashes / 53 unbound historical golden hashes**
verify. Latest aliases and historical retirement records are separate.
Gross cleanup savings differ from net checkout growth after builds:
**+829.98 MiB allocated /
+753.20 MiB apparent**. Exact measurements and deletion ledger:
`run13-disk.json`, `run13-disk-final-*.txt`, `run13-cleanup.json`.

### Run 12 continuation (2026-10-23; previous checkpoint)

Read the entire brief through **Update 6**, newest reports/messages and current
STATUS. Fresh scoreboard reproduces the run-11 baseline. Math provenance and
UpdateImport bookkeeping were already complete; no repeated implementation.
Changes remain uncommitted, with inherited edits preserved.

**TALA sequence edge publication: 2 runtime failures removed; +1 exact SVG,
zero regressions.** Pinned `d2layouts/d2talalayout/patch.go:158–239` publishes
only surviving edges after `internal/grouping/sequences.go:78–137` consumes
the defining step connections. Native `LayoutResult.removed_edges` now carries
that inventory through nested edge-ID remapping and cache restoration. Input
arena IDs stay stable, retaining each surviving edge's label/style/index.
Shared target JSON/hash/SVG export omits the removed connections; the legacy
SVG route validator still rejects unrelated missing routes. Native D2 JSON
publication now uses this explicit inventory as well.

| e2e outcome | run 12 start | qualified |
|---|---:|---:|
| exact SVG | 903 | **904** |
| expected error | 11 | **11** |
| target mismatch | 27 | **28** |
| runtime failure | 3 | **1** |

TALA `regression/nested_steps` changes **runtime failure → exact SVG**;
`stable/investigate` changes **runtime failure → target mismatch**. Its
remaining geometry difference is still non-passing. All prior SVG/error
matches survive, with identical full board inventories. The other runtime
failure, `real_world/jupyter_aws_eks`, is a separate nil dereference at
`labeling/guarded.rs:63`, reached by `reroute_grid_scope_external_edges` →
`place_new_edge_labels`. A bounded rerun confirms pinned Go matches its golden;
`run12-jupyter-panic.log` retains the Rust stack. The measured baseline thus
contained two step-edge runtime failures, not three.

Witness: `tests/regressions/e2e/tala-step-edge-removal.d2`. It fails before the
change and now produces exact normalized pinned-Go SVG. Tests also cover an
extracted grid cell, surviving connection metadata, result JSON round trips,
legacy deserialization, nested/cache ID remapping and explicit-removal versus
unrelated-missing-route admission.

Validation: **743 tests pass** (203 CLI, 100 layout, 76 native JSON, 264 render;
one existing native JSON test remains ignored). Final plain CLI build and
WASM consumer check pass. The check exposed an inherited missing
`RenderOptions.d2svg_hash_input` initializer in WASM; supplied `None`, preserving
its current renderer. This is not the S6 WASM renderer migration. Changed-file
formatting and `git diff --check` pass. Language scoreboard is unchanged:
**915 complete JSON matches / 445 editor source/error matches**, zero
regressions, using its unchanged runner. No fixture exclusions, lossy comparison
normalization, Go/JS implementation or ordinary-build diagnostics were added.

Evidence under `.reference/codex-runs/`: `run12-start.json`,
`run12-e2e-final.json`, `run12-e2e-final-delta.txt`,
`run12-language-qualified.json`, `run12-language-delta.txt`, test/build logs,
`run12-jupyter.json`, `run12-validation.json` and `report-run12.md`.
Latest scoreboard aliases were replaced atomically; prior run-11 reports and
the run-12 negative baseline remain retained. Source mappings updated in
layout/TALA/render SOURCE files.

Next, following brief Update 6: **S6 WASM SVG → d2svg**, then raster/PDF/PPTX
inputs using existing gates; afterward the **28 target mismatches** and the
separate Jupyter runtime failure. Historical language rows remain explicitly
non-passing as audited in run 11. Broader native/WASM/API and full D2 parity
remain incomplete.

### Run 12 disk audit (latest, 2026-10-23)

The interrupted report left this paragraph unfinished; retained run-12 raw
measurements show all ceilings passed: checkout **15.41 / 27.92 GiB allocated /
apparent**, `.reference` **6.73 / 17.90 GiB**, `target` **6.69 / 7.84 GiB**;
free **82.78 GiB**. Cleanup removed **204 KiB allocated**. Evidence:
`run12-disk-final-{allocated,apparent,free}.txt` and `run12-cleanup.json`.


### Run 11 continuation (2026-10-23; previous checkpoint)

Read the full brief through Update 5 and newest reports before continuing.
Fresh baselines reproduce 903 e2e SVGs and all 915 live complete language
goldens. Math provenance/notices were already completed in run 2.

**Editor source/errors: 426 → 445 live cases, +19 / zero regressions.**
Added native `update_import`, mapping original `d2oracle/edit.go:3337–3478`:
exact/directory path matching, spread deletion, field clearing, nested maps,
relative prefixes, selective-import components and contextual parse errors.
It parses/formats without opening imports or compiling. All 19 current
`TestUpdateImport/*` cases match, plus six pinned-Go boundary witnesses.

**Historical binding audit completed:** all 53 retained unbound goldens remain
non-passing. A fresh Go run confirms **38 names have no current test** (3 IR,
31 compiler, 4 editor); **15 UpdateImport graph names now have text-only tests**.
Four additional relative-path UpdateImport tests have no graph golden. The
harness measures their text independently and never substitutes it for a graph
match. Do not reconstruct historical inputs from expected outputs or repeat
the missing-hook investigation without new source evidence.

| tier | run 11 start | qualified | outstanding |
|---|---:|---:|---|
| parser complete JSON | 45 | **45** | 0 of 45 |
| IR complete JSON | 100 | **100** | 3 historical unbound of 103 |
| compiler complete JSON | 344 | **344** | 31 historical unbound of 375 |
| editor source/errors | 426 | **445** | 0 of 445 current tests |
| editor complete graph | 426 | **426** | 19 historical unbound of 445; 4 new text-only tests have no graph golden |

Both original and expanded language inventories report **+19 / 0 regressions**.
Complete JSON remains **915 matches**. E2e remains **903 SVG matches / 11
expected errors / 27 target mismatches / 3 runtime failures**, **0 regressions**;
reran the unchanged CLI executable. No comparison normalization, excluded
golden, ordinary-build tracing or Go/JS native implementation was introduced.

Validation: **61 editor tests + 11 harness tests pass**, including 19 current
text fixtures and six Go boundary witnesses. Fresh Go hooks reproduce the
entire **934-request** inventory exactly; all four selected Go test packages
pass. Formatting and `git diff --check` pass. Evidence:
`run11-{start,e2e-qualified,language-start,language-expanded-start,language-qualified}.json`,
both deltas, `run11-unbound-audit.json`, Go test events, native test logs,
`run11-validation.json` and `report-run11.md`. Current immutable input inventory
is `.reference/language-scoreboard-20261022/cases-v2.json`; the old inventory
remains retained. Latest scoreboard aliases updated. Changes are uncommitted.

Next: return to the live e2e runtime/geometry failures, then S6 exports and
later scoreboards. Broader native/WASM/API qualification remains open; these
finite corpora do not establish full D2 compatibility.

### Run 11 disk audit (2026-10-23)

All ceilings pass. Whole checkout **15.05 / 27.56 GiB allocated /
apparent**; `.reference/` **6.73 / 17.90 GiB**; `target/`
**6.33 / 7.48 GiB**; volume free **83.92 GiB**.
Hidden directories, .git and every hard-linked apparent path are included.
Exact measurements: `run11-disk.json` and `run11-disk-final-*.txt`.

`run11-cleanup.json` records **4,989 deleted files / 735.52 MiB
allocated**: completed Cargo codegen, idle Go action cache and duplicate hook
captures. Cargo locks held; deleted paths had no lsof holders/errors. Shared
module inputs, current executables, source and corpus bindings remain. No bulk
moved elsewhere. Fresh hooks matched all 934 retained requests before capture
retirement; post-cleanup checks verify 328 e2e inputs, 1,878 expected outputs,
930 bound language golden hashes and all 53 historical unbound hashes.
Gross savings differ from net checkout change: **-293.05 MiB
allocated / -282.87 MiB apparent**. No per-run compression
pass, following brief Update 3; .reference remains below 10 GiB allocated.

### Run 10 continuation (2026-10-23; previous checkpoint)

Read the complete brief through **Update 5**, newest reports/messages, STATUS,
PLAN and scoreboard workflow. Fresh baselines reproduced 903 e2e SVG matches,
344 compiler matches, 386 editor source/error matches and zero complete editor
graph matches. Followed the largest live gap; no commits or discarded inherited
changes.

**Editor complete graphs: 0 → 426 / 445, +426 / zero regressions.**
**Editor source/errors: 386 → 426 / 445, +40 / zero regressions.**
All **915 live language goldens** now match complete JSON. The **53 historical
unbound goldens** remain non-passing (3 IR, 31 compiler, 19 editor).

- One native graph projection: editor `recompile` returns
  `compiler::source_graph_json` from the post-edit compile, following original
  `d2oracle/edit.go:315–324`. Changed edits format/reparse at the original path
  with import context; no-op Move/Reconnect keeps its original AST ranges.
  Final formatting precedes graph serialization (`edit_test.go:8327–8347`),
  including quoted-string raw text in `TestSet/label_primary`.
- Create now uses compiled object/edge uniqueness and shared Set insertion:
  `edit.go:31–77,2650–2731`. Nested scopes, edges, imports and boards match all
  **36 live Create cases**. Empty scalar boards (`TestCreate/add_layer/4,5`)
  use an empty IR map and materialized BaseAST, matching
  `d2compiler/compile.go:185–211,230–267`.
- Compiled Reconnect maps `edit.go:124–313` and `get.go:243–250`: board scopes,
  no-op endpoints, preservation of former endpoint nodes, chain splitting and
  scope-relative endpoint references. All **24 live Reconnect cases** match,
  including `contained`, `scenarios-chain`, and nonexistent endpoints/edges.
  Its unwrapped error/debug forms remain distinct from wrapped edit errors.
- Existing AST-only helpers and serialized source-edit responses retain their
  contracts. Native compiler-backed results expose the complete graph directly;
  the scoreboard consumes those results without per-case adapters.

| tier | run 10 start | qualified | outstanding |
|---|---:|---:|---|
| parser complete JSON | 45 | **45** | 0 of 45 |
| IR complete JSON | 100 | **100** | 3 unbound of 103 |
| compiler complete JSON | 344 | **344** | 31 unbound of 375 |
| editor source/errors | 386 | **426** | 19 unbound of 445 |
| editor complete graph | 0 | **426** | 19 unbound of 445 |

Full language delta: **466 tier improvements / zero regressions**, not 466
unique cases. E2e remains **903 exact SVG / 11 expected errors / 27 target
mismatches / 3 runtime failures**, **zero regressions**. The unchanged run-9
CLI executable was rerun and its hash verified; editor is outside its dependency
path. No Go/JS implementation, comparison normalization, excluded golden or
ordinary-build trace was introduced.

Validation: **59 editor tests + 7 harness tests pass**. New tests compare all
426 complete graph/source/error results in
`crates/d2-rs-editor/tests/fixtures/editor-graphs-v090.json`, plus generated
Create keys and legacy serialization. Existing suites retain all 224 Delete/Set,
630 Move and 25 Rename inputs. All **426 fixture outputs/hashes**, **915 live
language expected goldens** and **1,878 e2e expected outputs** verify after
cleanup. Changed-file formatting and `git diff --check` pass.

Evidence: `.reference/codex-runs/run10-{start,e2e-qualified,language-start,
language-qualified}.json`, `run10-{e2e,language}-delta.txt`, per-cluster deltas,
final test/build logs, `run10-validation.json` and `report-run10.md`. Both latest
scoreboard aliases updated. Next: bind the 53 unbound language goldens/current
UpdateImport tests; e2e runtime/geometry and S6 exports remain open. These finite
corpora do not establish full D2 or editor/API/WASM parity.

### Run 10 disk audit (2026-10-23)

All ceilings pass. Whole checkout **15.34 / 27.84 GiB allocated /
apparent**; `.reference/` **7.02 / 18.17 GiB**; `target/`
**6.33 / 7.48 GiB**; volume free **85.83 GiB**.
Hidden directories, .git and every hard-linked apparent path are included.
Exact commands/KiB: `run10-disk.json` and `run10-disk-final-*.txt`.

`run10-cleanup.json` records **503 deleted files / 536.56 MiB allocated**:
completed run-10 codegen/intermediate files and four superseded run-9 editor
executables with dependency records. Current tested executables remain. Old
executables had no lsof holders/errors and no retained manifest/binding
references; their hashes, replacements and rebuild commands are in the ledger.
Cargo locks were held during deletion. No corpus inputs/outputs, source or
inherited scratch witness was deleted; no bulk moved elsewhere. Post-cleanup
language/fixture/e2e expected-output verification passes. Gross savings differ
from net checkout growth: **+34.20 MiB allocated /
+33.98 MiB apparent**. No per-run compression,
following brief Update 3; .reference remains below 10 GiB allocated.

### Run 9 continuation (2026-10-23; previous checkpoint)

Followed the entire brief (including Updates 1–4), newest run-7/run-8 reports,
STATUS and the scoreboard workflow. Fresh baselines reproduce **903 exact e2e
SVGs / 335 compiler goldens**. Completed Update 4's compiler cluster:
**335 → 344 compiler complete JSON matches, +9 / 0 regressions**. All 344
live compiler cases now match; the **31 unbound historical goldens remain
non-passing**. Existing changes preserved; no commits.

- Four imported-board AST results (`TestCompile/{glob-connection-steps,
  import-link-layer-4,import-link-underscore-2,import-link-underscore-3}`)
  now materialize BaseAST before serializing the root, preserving each
  import as a spread node: original `d2compiler/compile.go:185–211,215–267`.
  Moved the native pre-measurement graph projection from the golden example
  into `compiler::source_graph_json`; the compiler/editor share
  `find_board_base_ast` rather than duplicate board-import logic.
- `TestCompile2/boards/isFolderOnly`: compile a scalar board through an empty
  IR map and apply its primary label, matching `compileBoardsField:186–201`.
- `TestCompile/text-transform`: retain raw compiled object/edge labels from
  `compileLabel:473–508`; measurement still applies text transforms.
- `TestCompile/legend`: emit original legend boxes (10,10; 100×100), omit
  transparent objects and retain the two-point edge routes, matching
  `compileLegend:127–175`.
- `TestCompile/var-not-color`: theme override diagnostics use the entire
  LastPrimaryKey range (`compileThemeOverrides:1552–1556`), not a key-path
  reference range. `TestCompile2/vars/config/data`: keep config data on the
  separate Config result (`Compile:63–67,compileConfig:1482`); its measured
  target representation is preserved.

| tier | run 9 start | qualified | outstanding |
|---|---:|---:|---|
| parser complete JSON | 45 | **45** | 0 of 45 |
| IR complete JSON | 100 | **100** | 3 unbound of 103 |
| compiler complete JSON | 335 | **344** | 31 unbound of 375 |
| editor formatted source/errors | 386 | **386** | 23 mismatches, 17 unsupported, 19 unbound of 445 |
| editor complete graph | 0 | **0** | 426 unsupported, 19 unbound of 445 |

Full language delta **+9 / 0 regressions**. Fresh native CLI rebuild retains
**903 SVG matches, 27 target mismatches, three runtime failures and 11 expected
errors**, **0 regressions**. No comparison normalization, excluded goldens,
Go/JS implementation calls or ordinary-build traces were introduced.

Validation: **58 language library tests + 2 new compiler contract tests**
(full nine-case JSON fixture and raw-vs-measured label behavior), **51 editor
tests** (44 library + 2 Delete/Set + 3 Move + 2 Rename), and **7 scoreboard
failure-gate tests** pass. The existing editor corpus covers 224 Delete/Set,
630 Move and 25 Rename inputs. Changed-file formatting and `git diff --check`
pass. Full results from pinned pristine Go goldens, with original hashes, are
retained in `crates/d2-rs-language/tests/fixtures/compiler-source-graph-v090.json`.
The math notices/provenance requested in brief Update 1 were already completed
and verified in run 2 (141 math tests); they were not redone.

Evidence: `.reference/codex-runs/run9-{start,e2e-qualified,language-start,
language-qualified}.json`, `run9-{e2e,language}-delta.txt`, final build/test
logs and `run9-validation.json`. Both latest scoreboard aliases updated.
Next per Update 4: use the shared compiler graph projection for native editor
results (426 unsupported graph cases), preserving the edited AST/reference
ranges rather than reparsing formatted text. Create/Reconnect source failures,
53 unbound language goldens/current UpdateImport binding, e2e runtime/geometry
and S6 exports remain open. No full D2 compatibility claim.

### Run 9 disk audit (2026-10-23)

All ceilings pass. Whole checkout **15.31 / 27.81 GiB allocated /
apparent**; `.reference/` **7.02 / 18.17 GiB**; `target/`
**6.30 / 7.45 GiB**; volume free **86.98 GiB**.
Hidden paths, .git and every hard-linked apparent path are included. Exact
commands/KiB: `run9-disk.json`, `run9-disk-final-{allocated,apparent,free}.txt`.

`run9-cleanup.json` records **491 deleted files / 429.00 MiB allocated**:
completed run-9 Cargo codegen files and superseded intermediate reports/logs.
No source, executable, corpus input/output or inherited scratch witness was
deleted; no bulk moved elsewhere. All **915** retained language input/expected
golden bindings verify original hashes after cleanup; the nine new fixture
outputs and current report/executable bindings verify. Gross cleanup savings
are distinct from net checkout growth: **+11.09 MiB allocated /
+10.94 MiB apparent**. No per-run compression pass,
following brief Update 3; .reference remains below 10 GiB allocated. All run-9
build/test/scoreboard sessions completed; both Cargo profile locks were held
exclusively during cleanup. No cross-session process-inventory claim.

### Run 8 continuation (2026-10-23; previous checkpoint)

Followed the complete brief, including Update 4, and reproduced both fresh
baselines before editing: **903 exact e2e SVGs** and **164 editor text/error
matches**. Added compiler-resolved `delete_object{,_with_options}` and
`set_object{,_with_options}`, preserving the existing root-only APIs.

- **Delete: 1 → 122 live text matches, +121 / 0 regressions.** Maps original
  `d2oracle/edit.go:937–1697`: resolve source references, rename child conflicts,
  hoist children, split edge chains, preserve surviving endpoints/table-column
  references, decrement indexed setters, remove reserved fields, update near
  targets, and append null overrides for imported/glob/inherited objects.
  Reduced examples retained in the fixture include deleting `p` from
  `p: {x}\nx` (hoist as `x 2`, retaining `x`) and deleting the middle node of
  `p -> y -> z` (preserve `p` and `z`).
- **Set: 1 → 102 live text/error matches, +101 / 0 regressions.** Maps
  `edit.go:80–129,327–935`: literal values and tagged block strings, compiled
  label/property references, nested insertion scopes, chained/indexed edges,
  arrowhead/style maps, class/variable/glob setters, imports and board-local
  overrides. `x` plus Set `x.style.opacity = "0.4"` becomes
  `x: {style.opacity: 0.4}`. Writable imported board maps preserve their
  spread import, matching `d2compiler/compile.go:231–267` (`_findFieldAST`).

All **224 live Delete/Set source/error cases** match. The two old goldens with
no live hook input remain unbound; complete editor graph results remain
unsupported. No golden exclusions, comparison normalization or diagnostic
traces were added.

| tier | run 8 start | qualified | outstanding |
|---|---:|---:|---|
| parser complete JSON | 45 | **45** | 0 of 45 |
| IR complete JSON | 100 | **100** | 3 unbound of 103 |
| compiler complete JSON | 335 | **335** | 8 mismatches, 1 unsupported, 31 unbound of 375 |
| editor formatted source/errors | 164 | **386** | 23 mismatches, 17 unsupported, 19 unbound of 445 |
| editor complete graph | 0 | **0** | 426 unsupported, 19 unbound of 445 |

Full language delta: **+222 / 0 regressions**. E2e retains **903 SVG matches,
27 mismatches, three runtime failures and 11 expected errors**, **0 regressions**.
Editor changes are outside the CLI dependency graph; its unchanged scored
binary was rerun. Evidence in `.reference/codex-runs/`:
`run8-{start,e2e-qualified,language-start,language-qualified}.json`,
`run8-{language,e2e}-delta.txt`, and `run8-validation.json`.
`tests/fixtures/delete-set-v090.json` in d2-rs-editor binds every live pinned
Delete/Set request and source/error result, with an exact inventory assertion.
The affected editor suites pass: **44 library tests + 2 Delete/Set corpus tests
(224 cases) + 3 Move corpus tests (630 cases, including eight explicit safe
rejections of source panics) + 2 Rename tests (25 upstream cases plus generated
names)**. Seven scoreboard failure-gate tests, editor formatting checks and
`git diff --check` pass. No full compatibility claim; edits are uncommitted.

Next per Update 4: the **eight compiler mismatches**, then one shared native
editor graph projection for the 426 live graph cases. Remaining editor text
failures are Create/Reconnect; 53 unbound language goldens and current
UpdateImport text binding also remain open. E2e geometry/runtime failures and
S6 exports are unchanged.

### Run 8 disk audit (2026-10-23)

All ceilings pass. Whole checkout **15.29 / 27.80 GiB allocated /
apparent**; `.reference/` **7.02 / 18.17 GiB**; `target/`
**6.29 / 7.44 GiB**; volume free **86.98 GiB**. Hidden paths,
.git and every hard-linked apparent path are included. Exact commands/KiB:
`run8-disk.json` and `run8-disk-final-{allocated,apparent,free}.txt`.

`run8-cleanup.json` records **357 deleted files / 290.12 MiB allocated**:
this run's completed Cargo codegen temporaries and superseded intermediate
reports/logs. Required inputs/outputs, source, executables and inherited
scratch witnesses remain. No bulk moved elsewhere. Gross savings are distinct
from net checkout growth: **+12.70 MiB allocated /
+12.26 MiB apparent** after builds. No per-run compression
pass, following brief Update 3; .reference remains below 10 GiB allocated.
All run-8 build/test/scoreboard sessions completed; idle Cargo locks were
verified during cleanup. No cross-session process-inventory claim is made
(the sandbox denies ps/pgrep).

### Run 7 continuation (2026-10-23; previous checkpoint)

Followed the brief's Update 3: measure language parity after qualifying the
unfinished e2e handoff. The saved run-4 score binary still gave **901 exact SVG
matches**. A fresh source build, including unreported run-5 Dagre edits,
gives **903 SVG matches / 27 target mismatches / 3 runtime failures / 11
expected errors**: **+2 fixed / 0 regressions**, Dagre `themes/{origami,terminal}`.
Rebuilding with the run-7 link fix removed independently reproduces 903;
restoring it retains 903 with **0 regressions**. The inherited fix merges a
short endpoint segment after the final outside-label intersection, mapping
`d2graph/layout.go:449–453,531–535` (`TraceToShape`). Its existing reduced
geometry unit test is `final_trace_merges_a_short_segment_at_an_outside_container_title`.
Reports: `run7-start.json`, `run7-current-source-baseline.json`,
`run7-e2e-after-links.json`. The two inherited improvements are not attributed
to the new compiler change.

Added `scripts/{dump-language-scoreboard,language-scoreboard,
language-scoreboard-delta}.py` and the Rust-only `language-scoreboard` example.
The Go dump runs env-gated hooks in the pinned instrumentation test packages;
all **915 live captured cases** reproduce their complete retained goldens.
All **968 retained goldens** remain inventoried. **53 unbound retained
names** (3 IR, 31 compiler, 19 editor) have no current hook input and never
count as passing. These include old UpdateImport graph goldens whose current
tests assert source text rather than writing graph goldens; the current dump
does not yet bind that operation. JSON comparisons preserve ranges, complete
references,
source order, diagnostics and null/empty distinctions. Raw input hex preserves
the UTF-16 test bytes. Native failures, missing/corrupt output and timeouts fail.
The editor's complete graph tier is explicitly unsupported; formatted source
and errors are measured separately. The compiler adapter uses source records
before SetDimensions; its remaining projection gaps remain failures.

| tier | first measured | after fixes | outstanding |
|---|---:|---:|---|
| parser complete JSON | 45 | **45** | 0 of 45 |
| IR complete JSON | 100 | **100** | 3 unbound of 103 |
| compiler complete JSON | 329 | **335** | 8 mismatches, 1 unsupported, 31 unbound of 375 |
| editor formatted source/errors | 143 | **164** | 196 mismatches, 66 unsupported, 19 unbound of 445 |
| editor complete graph | 0 | **0** | 426 unsupported, 19 unbound |

- Compiler source records now clear rejected board links just as render styles
  do. Original `d2compiler/compile.go:1334–1374` clears the original object's
  link for missing/non-board/self targets. Fixes six complete compiler goldens:
  `TestCompile/{link-board-not-found-1,link-board-not-found-2,no-self-link,
  link-board-underscore-not-found,link-board-not-board,link-beyond-import-root}`,
  **+6 / 0 regressions**. Reduced source-record witness:
  `x.link: layers.missing`, with retained remote-link control.
- Added compiler-resolved `rename_object{,_with_options}`. Original
  `d2oracle/edit.go:1699–1748` builds the same-parent destination, quotes a
  literal final-component name, handles indexed-edge arrows and calls the
  shared `move(..., false)` implementation. Reusing the native translated
  Move path preserves declarations, edge/near references, suffixes and board
  write restrictions. All **25 live TestRename formatted-source/error cases**
  now match: **+21 / 0 text-tier regressions**. The previous root-only API's
  contract remains unchanged. Fixture: `tests/fixtures/rename-v090.json` in
  d2-rs-editor; complete editor graph parity is not claimed.

Language reports: `.reference/codex-runs/run7-language-{start,after-links,
after-rename,qualified}.json`; deltas: `run7-{link,rename,language,e2e}-delta.txt`. Cases are retained
once at `.reference/language-scoreboard-20261022/cases.json`. Build, dump and
score commands are documented in SCOREBOARD-WORKFLOW. Seven harness failure
gate tests pass. Native verification: **58 language library + 44 editor library
+ 2 Rename integration + 107 Dagre library tests pass**, zero failures. The
Rename integration test runs all 25 source/error fixtures. All four pinned
Go test packages pass. `git diff --check` and formatting checks for changed
editor files pass. A broad Cargo test run was stopped during relinking, before
execution, to bound storage; only the listed suites are claimed. Complete
commands/results are in `run7-validation.json`. No diagnostics were added to
ordinary builds.

Next: editor Delete/Set are the largest measured live failure clusters (101
and 72 text cases). Finish their compiler-resolved contracts and expose native
editor graph results before claiming the 445 graph goldens. Compiler failures
include imported-board AST projections, unmeasured text transforms, a variable
error range, legend geometry, folder-only scalar boards and source config data.
Do not turn those adapter gaps into passing exclusions. E2e's three TALA runtime
failures, 27 geometry mismatches, S6 WASM/export migration and the later language/
export scoreboards remain open. No full D2 parity claim; changes are uncommitted.

### Run 7 disk audit (historical, 2026-10-23)

All ceilings pass. Whole checkout **15.28 / 27.78 GiB allocated / apparent**;
`.reference/` **7.02 / 18.17 GiB**; `target/` **6.28 / 7.43 GiB**;
volume free **87.36 GiB**. Measurements include hidden directories,
.git and every hard-linked apparent path. Exact commands/KiB: `run7-disk.json`.

`run7-cleanup.json` records **7,020 disposable files retired,
1.96 GiB allocated**: completed/interrupted Cargo codegen temporaries,
raw Go dumps replaced by canonical cases plus strict revalidation, superseded
harness drafts and the disposable pre-link baseline executable. The previous
score alias is explicitly retired by hash and replaced with the scored binary.
No bulk moved outside the checkout. Gross savings are distinct from the net
checkout change after builds: **+0.40 GiB allocated / +0.36 GiB apparent**.
No per-run compression pass, following Update 3; .reference remains below
10 GiB allocated. No build/test/oracle process from this run remains active.

### Brief continuation (2026-10-22)

Fresh baseline reproduced **864 SVG matches / 66 target mismatches / 3 runtime
failures / 11 expected errors**, zero change from the handoff. Field clustering
found route differences on 41 boards, shape positions on 21, edge label
percentages on 13 and edge label positions on 11.

Removing Dagre's synthetic nested-container route override fixes
`stable/direction`: **865 SVG matches, 65 target mismatches, 3 runtime failures**,
**+1 fixed / 0 regressions** across the full 944 applicable boards. Original Go
`d2layouts/d2dagrelayout/layout.go:267–380` preserves the fitted route through
`Edge.TraceToShape` and cubic conversion; the removed Rust override replaced
that correct curve with a new border-ending route and changed its lane.
Reduced witness: `tests/regressions/e2e/dagre-nested-container-route.d2`.
All 107 Dagre tests pass. Reports and field diffs are in
`.reference/codex-runs/`; the handoff baseline remains preserved there.

### Run 2 continuation (2026-10-22)

The fresh run starts at **868 SVG matches / 62 target mismatches / 3 runtime
failures / 11 expected errors**. It reproduces the unfinished nested-grid
label handoff: restoring label state with each extracted route fixes
`real_world/jupyter_aws_eks` (ELK) and `real_world/jupyter_k8s_oidc` (Dagre,
ELK), **+3 fixed / 0 regressions** over the 865-match route checkpoint.
Go `d2layouts/d2layouts.go:732–750` injects the original edge objects,
including their grid/sequence label state. Reduced witness:
`tests/regressions/e2e/nested-grid-edge-label.d2`.

Target export now reads TALA's committed edge-label percentages instead of
pre-layout source values. Presence uses `edge_label_percentage_commits` and
`edge_label_percentage_present`, preserving explicit zero and 0.5. Source:
`d2layouts/d2talalayout/patch.go:371–397` (`buildEdgePatch`) and
`d2exporter/export.go:461–466` (`toConnection`, float32 conversion).
The full scoreboard improves to **878 SVG matches / 52 target mismatches /
3 runtime failures / 11 expected errors**, **+10 fixed / 0 regressions**.
Fixed TALA boards: `real_world/{jupyter_k8s_oidc,leios_simulator,
lion_reader_frontend,queue_workers}`, `stable/{chaos2,dagre_spacing,
edge-label-overflow,overlapping_image_container_labels}`, and
`txtar/{c4-person-shape,edge-special-text}`. Reduced witness
`tests/regressions/e2e/tala-edge-label-percentage.d2` changes target hash
115923749 → **3623621977**, equal to pinned Go; normalized SVG also matches.
The root cause was export state, so this changes no layout algorithm.

Grid image cells now retain their icon position as well. Go's
`d2layouts/d2grid/layout.go:185–200` checks `Icon != nil`, including image
shapes, while Rust used the image-excluding `HasIcon` condition. Keeping
geometry spacing unchanged but publishing that position fixes
`regression/grid_image_label_position` and `themes/3d-sides` under ELK and
TALA: **+4 fixed / 0 regressions**. The reduced
`tests/regressions/e2e/grid-image-icon-position.d2` now matches pinned Go's
normalized SVG under all three engines.

| checkpoint | SVG matches | target mismatches | runtime failures | expected errors |
|---|---:|---:|---:|---:|
| handoff baseline | 864 | 66 | 3 | 11 |
| route override removed | 865 | 65 | 3 | 11 |
| run 2 start / nested labels | 868 | 62 | 3 | 11 |
| committed percentages | 878 | 52 | 3 | 11 |
| grid image icon positions | **882** | **48** | **3** | **11** |

All **944 applicable boards** remain accounted for; 9 measured-text boards,
3 empty CLI outputs and 10 no-golden boards remain separately inapplicable.
Run 2 adds **14 SVG matches / 0 regressions** over its fresh start; the
complete continuation adds **18 / 0** over the handoff baseline.

The brief's math provenance is retained and its complete pinned LICENSE,
NOTICE and additional MIT license now ship in `crates/d2-rs-math/licenses/`.
`cargo test --profile oracle -p d2-rs-math`: **141 passed, 0 failed**.
Render suite: **263 passed**; layout suite: **99 passed**, including all
three engines for the nested-grid edge labels and image icon positions.
The new layout test's measured-size/default-label fixture corrections are
recorded in `run2-validation.json`. `git diff --check` passes;
`cargo fmt --all -- --check` reports inherited formatting differences, which
are left untouched.
Evidence: `.reference/codex-runs/{run2-start,after-percentage,
after-grid-icons,percentage-witness,grid-image-witness,run2-evidence,
run2-validation}.json`; the original baseline and intermediate route/
nested-label checkpoints remain available. `run2-evidence.json` hashes all
328 case inputs, changed implementation files, regression witnesses and
both the scored binary and diagnostic negative baseline. The latest
scoreboard is copied to `.reference/e2e-scoreboard-20261022/latest.json`.

### Run 3 continuation (2026-10-22)

Fresh scoreboard: **882 SVG matches / 48 target mismatches / 3 runtime
failures / 11 expected errors**, identical to run 2. Pinned Go reproduces
all 12 selected ELK goldens (`run3-go-reference.json`).

Cross-hierarchy ELK sections now keep their flow coordinates relative to the
edge's common container. They previously used root coordinates; the codec then
added that container's origin a second time. This left node boxes correct but
shifted internal routes. Source: elk-go v0.2.0 `internal/layered/compound.go`
`joinHierarchyEdges` / `changeGraphCoordinates` (reference-graph conversion),
`transferrer.go:255–376` (`applyEdge` / `edgeLayoutOffsets`, original container
coordinates), and D2 `d2layouts/d2elklayout/layout.go:508–533` (container-origin
addition). The same correction applies to hierarchy junctions.

Reduced witness: `tests/regressions/e2e/elk-nested-edge-container.d2`.
Its complete normalized SVG matches Go in **all four directions**; DOWN's
negative/positive target hashes are **592546808 → 1902065081**.
A complete four-direction Go wire fixture is retained in
`crates/d2-rs-elk/src/fixtures/original_nested_edge_container.json`.
The full scoreboard improves to **893 SVG matches / 37 target mismatches /
3 runtime failures / 11 expected errors**, **+11 fixed / 0 regressions**.
Fixed ELK boards: `regression/dagre-disconnect`,
`stable/{chaos2,dagre_spacing,dagre_spacing_right,elk_shim,
overlapping_image_container_labels}`, `themes/{dark_terrastruct_flagship,
origami,terminal,terminal_grayscale}`, and `txtar/shape-animate`.
Evidence: `.reference/codex-runs/{run3-start,run3-after-flow,
run3-flow-witness,run3-current-diffs,run3-go-reference}.json`.
`cargo test --profile oracle -p d2-rs-elk`: **236 passed**, including the
four-direction fixture.

Dagre now carries its normalized raw dummy-chain routes through the adapter
whenever the translated border model is present. It previously selected that
source path only for multiple root containers, reconstructing different lanes
for single-container graphs. Original D2 `d2dagrelayout/layout.go:210–273`
reads every Dagro edge's `points`, clips them once to authored boxes, then
carries them through spacing and fitPadding. Reduced witness:
`tests/regressions/e2e/dagre-single-container-dummy-lanes.d2`; its normalized
SVG matches Go (target **2405017235**). This fixes Dagre `stable/elk_shim`,
`themes/dark_terrastruct_flagship`, and `txtar/c4-person-shape`:
**896 SVG matches / 34 target mismatches / 3 runtime failures / 11 expected
errors**, **+3 fixed / 0 regressions**, **+14 / 0** for run 3.
`run3-after-dagre-lanes.json` and `run3-dagre-reference.json` retain the full
delta and nine checked Go goldens. `cargo test --profile oracle -p d2-rs-dagre`:
**107 passed**. CLI regression assertions now cover both reduced witnesses. The initial CLI
suite found an inherited witness-test flag error (`--tala-seeds` passed to the
internal helper); correcting it to `--seeds=1` gives **202 passed / 0 failed**.
The public CLI parser and the scoreboard arguments are unchanged.
S6 and language scoreboards remain outstanding.

### Run 4 continuation (2026-10-22)

Fresh scoreboard reproduces run 3: **896 SVG matches / 34 target mismatches /
3 runtime failures / 11 expected errors**. Pinned Go reproduces all four
Jupyter Dagre/ELK goldens (`run4-jupyter-reference.json`).

Restore the nested grid crossing callback under Dagre/ELK as well as TALA.
Original `d2layouts/d2layouts.go:330–382` routes each extracted graph's
crossings after reinjection; DefaultRouter also commits the label position.
Rust's root-grid path only called this for TALA, and ELK's object-grid path
returned before the callback. Geometry happened to match, but label state
was absent. Reduced witnesses: `nested-grid-crossing-label.d2` (five lines,
root grid) and its object-grid variant without the first line. The complete
root-grid SVG matches pinned Go under all three engines. Target hashes:
Dagre **4097949319 → 358427907**, ELK **2353163231 → 1856303271**;
TALA remains **2400284554**.

The full scoreboard reaches **899 SVG matches / 31 target mismatches /
3 runtime failures / 11 expected errors**, **+3 fixed / 0 regressions**:
`real_world/jupyter_k8s_oidc` (Dagre/ELK) and `real_world/jupyter_aws_eks`
(ELK). Intermediate root-grid checkpoint: 898 matches, +2 / 0; the object
callback adds +1 / 0. No exclusions or normalization changed. Reports:
`.reference/codex-runs/{run4-start,run4-after-labels,run4-after-grids,
run4-label-witness,run4-object-label-witness}.json`.

Dagre also no longer intersects compound-edge rectangles a second time
between the final rectangle trace and the silhouette trace. Source:
`d2graph/layout.go:487–499, 575–587` and
`d2layouts/d2dagrelayout/layout.go:348–351`. The extra rounded-delta
intersection shifted the ray by one pixel. Reduced witness:
`tests/regressions/e2e/dagre-compound-shape-trace.d2`; complete normalized SVG
matches Go, target **2161879430 → 2428849042**. This fixes Dagre
`themes/terminal_grayscale` and `txtar/shape-animate`: **901 SVG matches /
29 target mismatches / 3 runtime failures / 11 expected errors**, **+2 / 0**
for this change and **+5 fixed / 0 regressions** in run 4. Pinned Go reproduces
all five checked Dagre endpoint-cluster goldens (`run4-dagre-reference.json`).
Origami and Terminal remain failures. No geometry-tracing diagnostics were
added to ordinary builds. `cargo test --profile oracle -p d2-rs-dagre
-p d2-rs-layout` passes **107 + 99 tests**; the CLI suite passes **202 tests**
under each of its two Cargo targets, including the new source-derived hashes.
`git diff --check` passes; inherited unrelated formatting remains untouched.
`run4-validation.json` records commands and complete result lines.

`run4-qualified.json` is now the latest scoreboard. Its canonical binary alias
has the exact scored SHA-256. `run4-source-bindings.json` and `run4-only.patch`
isolate this run from inherited changes; source objects contain no executables.
Complete source/native witness captures and negative baselines are retained in
`witness-inputs/run4`. `run4-retirement.json` separately records historical
binary hashes, rebuild recipes and current aliases. After cleanup, all 328
case inputs, 1,878 expected outputs and witness/source-object hashes verify.

### Run 4 disk audit (historical, 2026-10-22)

All ceilings pass: whole checkout **14.58 / 27.15 GiB allocated / apparent**;
`.reference/` **6.69 / 17.85 GiB**; `target/` **5.90 / 7.11 GiB**;
volume free **88.50 GiB**. Hidden directories, .git and every hard-linked
apparent path are included. `run4-cleanup.json` records **3,544 deleted files**,
**1726.38 MiB allocated**: completed Cargo codegen/Thin-LTO temporaries,
idle Go action-cache entries, superseded binary aliases and disposable run-4
captures/build logs/helper. Pinned inputs, expected outputs, published package,
source objects and negative witness captures remain. `run4-retirement.json`
records replaced binary hashes and rebuild recipes; historical aliases must
not be treated as their old bytes.

`run4-compression.json` records **122.29 MiB** saved with Applesauce 0.5.30,
verified LZFSE compression on six idle single-link executables and 11 witness
captures. Hashes, modes, executability, ownership and non-compression xattrs
are unchanged; verified APFS clone backups were removed immediately.
Rollback is `applesauce decompress --verify -- <exact recorded path>`.
Gross deletion/compression savings: **1.81 GiB**, distinct from net checkout
change after builds: **-0.18 GiB allocated / -0.16 GiB apparent**.
No bulk moved outside this checkout. Exact measurements and raw audit output:
`run4-disk.json` and `run4-disk-{allocated,apparent,free}.txt`.
All 328 case inputs and 1,878 expected outputs verify after cleanup. No build,
test or oracle process remains active. Changes are uncommitted.

### Run 3 disk audit (2026-10-22)

All ceilings pass: whole checkout **14.77 / 27.31 GiB allocated / apparent**;
`.reference/` **6.68 / 17.85 GiB**; `target/` **6.09 / 7.28 GiB**;
volume free **88.12 GiB**. Whole checkout includes hidden directories and
.git; apparent size counts every hard-linked path. `run3-cleanup.json` records
5,762 retired action-cache/codegen/snapshot/scratch files,
**2474.44 MiB allocated**. `run3-compression{,-cli}.json` records verified
LZFSE savings of **137.95 MiB**, unchanged hashes/modes/owners/xattrs,
and immediate backup removal. No active or hard-linked file was rewritten.
Gross savings **2.55 GiB**, distinct from whole-checkout net change after
builds: **-1.68 GiB allocated / -1.59 GiB apparent**. No bulk moved
outside the checkout. Historical binary paths are explicitly retired in
`run3-retirement.json`; current canonical score/trace aliases verify separately.
All 328 case inputs, 1,878 expected target/SVG outputs and before/after source
objects verify; no corpus input/output closure was retired. Source objects
contain no executables. `run3-evidence.json`, `run3-output-bindings.json`,
`run3-validation.json` and `run3-disk.json` bind the completed checkpoint.

### Run 2 disk audit (2026-10-22)

All ceilings pass: whole checkout **16.45 GiB allocated / 28.90 GiB
apparent**; `.reference/` **7.08 / 18.23 GiB**; `target/` **7.38 / 8.48 GiB**;
volume free **90.03 GiB**. Hidden directories, `.git/` and every hard-linked
apparent path are included. `run2-cleanup.json` records 167 retired LTO/scratch
files, **397.65 MiB allocated**; `run2-compression.json` records verified
Applesauce LZFSE savings of **267.75 MiB**, preserving byte hashes, modes,
ownership and other xattrs. Temporary clone backups were removed. Source
witnesses remain under `witness-inputs/` with replacement hashes; no retained
corpus closure or immutable content-store object was deleted. Whole-checkout
net change after builds: **+5.91 MiB allocated / +188.51 MiB apparent**;
**665.40 MiB** gross savings are distinct from that net change. Final commands
and exact KiB: `.reference/codex-runs/run2-disk.json`. No active build remains.

`scripts/d2svg-golden.py` renders every upstream `board.exp.json` with the
Rust d2svg port (crates/d2-rs-render/src/d2svg/) and byte-compares it with
`sketch.exp.svg` (952 golden boards). `--go-oracle` renders the same JSON with
the pinned Go d2svg (`../d2-v0.9.0-trace/cmd/d2rs-svgrender`, built to
`.reference/bin/d2svg-golden-oracle`), so a golden that pinned Go itself
cannot reproduce is reported as stale and never counted as a match.

| run | svg-match | stale golden | unported (explicit errors) | mismatch |
|---|---|---|---|---|
| first core run | 638 | – | 260 | 54 |
| + Go shortest-float ties, net/url escaping | 688 | 4 (Rust == pinned Go) | 260 | 0 |
| + gradients, paper/grain, connection icons, legend | 720 | 6 (4 match Go, 2 rejected by Go too) | 220 | 6 (LaTeX, in progress) |
| + Markdown, code, LaTeX, sketch, positioned tooltips | **939** | 13 (11 match pinned Go, 2 rejected by Go too) | **0** | **0** |

**Every one of the 952 golden boards is accounted for**: 939 byte-identical
to the golden, 11 byte-identical to pinned Go where the golden is stale, and
2 whose JSON pinned Go cannot decode either. Ported clusters: native Markdown
(`d2svg/markdown*`, textmeasure LayoutMarkdown/SVG with x/image sfnt
fixed-point metrics), code (`code.rs` + d2-rs-text chroma_style/Bash),
LaTeX (`latex.rs` + d2-rs-math mathjax-go path), sketch (`sketch.rs` + the
rough-go Generator in d2-rs-sketch; a sketch-forced sweep over all 952 boards
matches pinned Go on 947, the rest being the errors above), positioned
tooltips (`tooltip.rs`).

Exact-output dependencies landed for this work:
- `crates/d2-rs-goflate`: Go 1.27 compress/flate + zlib writer, byte-exact on
  869/869 (case, level) pairs against Go 1.27.0. (Go 1.27 replaced the
  encoder; `/usr/local/go` is 1.23.4 and is not the oracle.)
- `crates/d2-rs-text` `subset_font`/`woff`/`d2fonts`: lib/font UTF8CutFont,
  Sfnt2Woff and GetEncodedSubset; 180/180 Go-oracle cases and 518/518 golden
  embedded fonts.
- `d2svg/{geo,label,shape,svgpath}`: lib/geo, lib/label, lib/shape, lib/svg
  path, bit-exact against Go oracles (ARM64 FMA sites taken from the compiler
  listing) and the golden SVG geometry.
- `d2_rs_core::go_json_float`: encoding/json float text with Go's
  ties-to-even shortest digits; used by both target hash serializers.

S6 has started. With `D2_RS_SVG_RENDERER=d2svg` the CLI renders SVG through
the port: `target_diagram_json` builds the public target (shapes,
connections, root, config, legend) from the same exporter-equivalent pieces
as the target hash, single boards go through d2svg.Render, and animated
multiboard output assembles the board tree and runs MasterID +
RenderMultiboard + d2animate.Wrap. End to end (D2 source → compile → layout →
SVG, all native), the initial `scripts/e2e-scoreboard.py --d2svg` run gave **864 of 944
applicable boards byte-identical to upstream `sketch.exp.svg` — every board
whose target hash matched**. At that checkpoint, the remaining 69 were layout divergences (66
target-hash mismatches, 3 TALA step-edge failures); SVG rendering no longer
loses any board. **d2svg is now the CLI's
default SVG renderer** (`D2_RS_SVG_RENDERER=legacy` selects the earlier
native renderer, which PNG/PDF/PPTX exports still build on). The default
scoreboard is now **933 svg-match / 0 regressions** (Claude continuation and run 18). Beyond the goldens, the CLI was
diffed byte-for-byte against the pinned Go CLI with `--dark-theme`,
`--theme`, `--scale`, `--center`, `--salt`, `--sketch`, `--pad`,
`--bundle` (local icon bundling), `--omit-version`, `--no-xml-tag`, ELK,
TALA, stdout, `--force-appendix` (appendix.Append ported), multiboard
directory output (per-board HashID covers descendant boards, as Go's
Diagram.Bytes does), animated output and `--target=x.*` subtree animation:
all identical. CLI tests: 201 pass; the 19 that assert the legacy dialect
now pin it explicitly, and `default_svg_output_is_byte_identical_to_upstream_golden`
guards the new default. WASM SVG switched in run 13. The native
raster/PDF/PPTX scene bridge remains on the earlier SVG/precision path;
further changes require a failing export case (brief Update 8).

## Upstream e2e scoreboard (2026-10-22) — primary measure

Work is now chosen and verified with the loop in
`docs/SCOREBOARD-WORKFLOW.md`. It replaces the earlier Codex-era practice of
qualifying internal TALA states without a failing board. The historical
sections below record that practice and remain regression evidence.

`scripts/e2e-scoreboard.py` scores every upstream e2e golden board
(`e2etests/testdata`, 328 cases; dagre for all, elk/tala unless `justDagre`)
through the native CLI with the e2e render options (pad 0, unbundled, TALA
seed 1). Cases are dumped from the pinned test package by the env-gated hook in
`../d2-v0.9.0-trace/e2etests/e2e_test.go` into
`.reference/e2e-scoreboard-20261022/cases/`. The pinned Go CLI reproduces the
goldens byte-for-byte on every checked failure (42 dagre target mismatches, 9
multiboard boards), so mismatches are genuine d2-rs divergences, not harness
noise. Tiers: error parity, target hash (D2's hash of the target diagram; equal
hash = equal shapes/connections/root/config), normalized SVG bytes.

| engine | initial scoreboard | now | applicable boards |
|---|---|---|---|
| dagre | 272 + 1 error | **317 + 7 errors** | 324 |
| elk | 256 | **308 + 2 errors** | 310 |
| tala | 221 | **308 + 2 errors** | 310 |
| **all** | **750** | **944 (933 SVG + 11 errors)** | **944** |

Inapplicable (never counted as passing): 9 measured-text cases (need injected
MText, not drivable through the CLI), 3 empty-diagram boards (the CLI writes no
file), 10 boards with no golden. **SVG byte tier: 933 / 944** through the
d2svg port; every current target match also matches SVG bytes.

Reports: `baseline-dagre.json`, `baseline-elk-tala.json`, `latest.json`
(updated to run 18's regression guard), and Go-CLI reference runs
`go-reference-*.json`. Run 2's reference sweep reproduces all 12 selected
golden boards with both CLIs (`run2-go-reference.json`).
Zero regressions against the previous report at every step
(`scripts/e2e-scoreboard-delta.py`).

### Fixes this session (each verified by scoreboard delta, 0 regressions)

- Target `isCurve` came from a `route.len() >= 4` guess instead of the edge's
  curve flag; Go never marks TALA routes as curves (d2exporter.toConnection
  copies edge.IsCurve). +59 boards.
- Exporter theme special rules missing from the target: ContainerDots,
  AllPaper, OuterContainerDoubleBorder (export.go applyTheme). Dashed shapes
  default to stroke B2 (Object.GetStroke). Target `bold` follows
  Object.Text + applyStyles even for Markdown/LaTeX labels, via a separate
  `SourceNodeStyle::target_bold`, so the regular measurement face is unchanged.
  +42.
- SQL-table columns measured in the graph font family; mono themes use Source
  Code Pro (d2graph.SetDimensions). DESCENDANT_EDGES feature check for dagre
  (plugin_features.go). pflag shorthand flags (`-l tala`, `-sw`, `-t=1`) were
  rejected by the render parser.
- `target-arrowhead.shape: none` exported as `triangle`; now `none`
  (d2target.ToArrowhead).
- DefaultRouter label position for straight-routed extracted edges
  (d2layouts.DefaultRouter / d2grid).
- Lifeline edge order follows LayoutNested's extraction/injection order
  (SaveOrder + InjectNested + d2near), in `order_lifelines_like_layout_nested`.
- Sequence diagram errors are reported verbatim (`no actors declared in
  sequence diagram`), and an endpoint with no actor ancestor fails as
  routeMessages/getCenter does. All 11 expected-error boards now match.
- TALA hierarchy eligibility skipped Go's tree-sentinel checks
  (hierarchy/eligibility.go Candidates/isSimpleCandidate/isEligibleContainer),
  so a container holding a retained branching tree became a hierarchy and its
  tree children were never placed. Witness
  `tests/regressions/e2e/tala-container-children-unplaced.d2`, regression
  test in `d2-tala-rs` (binpack_and_hierarchy.rs). Fixes
  stable/different_subgraphs.

Tests after these changes: `cargo test --profile oracle` for d2-rs-render,
-core, -language, -cli, -layout and d2-tala-rs: 2,614 passed, 0 failed, 24
pre-existing ignores. A re-run of core/layout/cli after the sequence-error
change gave 564 passed. One fixture
(`target_hash_matches_v090_simple_edge_witness`) now declares its dagre curve
flag explicitly instead of relying on the removed point-count guess.

### Remaining queue, current priorities

1. **Fix the largest measured root-cause cluster across the new tiers.**
   CLI, d2sketch and d2exporter are inventoried (runs 18–19). Runs 20–23 fix
   PDF inspectors, watch assertions, shorthand targets and PPTX structure;
   runs 24–29 fix watch/link inventories, sequence-layer order, GIF artifacts,
   directory inputs, target hashing and custom-font SVG embedding. Next: remaining PDF/PPTX representation/metadata differences,
   and named-subtree packaging/stdio (validation's stdout capture is closed in run 35)
   beyond the SVG/PNG file outputs fixed in run 33. Applicable CLI assertions already pass (run 30); run 31 closes
   the shared PPTX XML-content gaps. See the latest run section for exact counts.
   Preserve every existing assertion and artifact/golden match.
2. **Complete artifact contracts after assertions pass.** Five PDF cases
   now pass upstream assertions but remain byte mismatches in the conservative
   artifact tier. Decoded embedded pixels are unchanged; full document parity
   is not inferred from that observation.
3. **Regression guards:** all 933 applicable e2e SVGs and all 11 expected
   errors match; language remains qualified below. WASM SVG has 950 valid
   pinned-Go target matches. Go builds PNG/PDF/PPTX with d2scenebuild/d2raster;
   export changes still require a specific failing case, not a bridge rewrite.

The measured language queue remains closed: 915 complete JSON and all 445
current editor source/error cases match. The 53 historical unbound goldens
remain explicitly non-passing (run 11 audit); do not repeat that investigation
without new source evidence. Broader imports/globs/boards and WASM editing
are separate qualification work, not inferred from these finite corpora.

### Disk (2026-10-22, end of session)

All ceilings pass. Whole checkout **14.08 GiB allocated / 26.29 GiB
apparent**; `.reference/` **6.87 / 18.02 GiB**; `target/` **5.22 / 6.08 GiB**;
volume free **101 GiB**. Root cause of the target/ growth:
`[profile.oracle]` inherited dev's `incremental = true`. It is now disabled
(Cargo.toml). A verification rebuild left 75 MiB of LTO temporaries instead
of about 5.5 GiB, and those were removed too. During this session `target/` had grown to 22.4 GiB
(leaked thin-LTO `*.rcgu.o` temporaries: 6,721 files / 6.44 GiB; oracle
incremental cache 9.13 GiB; superseded rlib/rmeta variants 1.61 GiB; one stale
diagnostic-traces test binary 0.85 GiB). All were removed: about 18.0 GiB
recovered, all rebuildable. Intermediate scoreboard reports were removed;
baseline, latest and Go-reference reports remain (2.5 MiB total).

## Historical checkpoint: whole-compaction rollback (qualification in progress)

The working implementation now propagates operation contexts through sized and
sizeless compaction candidates, scoring and selected-position snapshots, and
shares normalized move limits. Selected snapshots preserve existing point
bindings without materializing new ones. Source comparison uses the pinned Go
v0.9.0 implementation; no external plugin work is included.

The current `.reference/tala-compaction-rollback-20261022/` investigation covers
7,196 source cases and 14,392 native invocations per build, including cancellation,
panic, point identity, routing state and move limits. The ordinary and diagnostic
TALA library suites pass (1,680 / 1,709 tests; 24 existing ignores each), and
format checking passes. The broader stage/raster replay, final retention audit,
compression and checkpoint sealing are not yet qualified at this commit. The
fully qualified checkpoint and disk figures below remain historical evidence;
they do not claim completion of this newer investigation or full D2 parity.

## Live compaction candidates and endpoint geometry

Compaction now preserves the distinction in original moves.go between latched
movement coordinates and live geo.Point operands. Occupancy and overlap scans
receive the actual candidate handle; equal-cost tie-breaking re-reads it after
callbacks. The spatial helpers retain their source-defined bounds/read order.

The route-bound sized edge scorer now reads both current and restored endpoint
boxes through the explicit canonical owner mapping after preparation callbacks.
Previously only restored originals used live owner geometry. A callback moving
the scored node onto its neighbor left ordinary geometry in a copied workspace
and missed the source NONE-orientation fast return, producing two extra polls.

Fresh source controls cover **1,074 cases**: twelve sized/sizeless, connected/
unconnected, fresh/node/sibling-alias baselines; one mutation at every reachable
first context poll and one unreachable post-completion poll; three replacement
points with/without persistent cancellation. **990 cases fire their mutation**.
All node/candidate coordinate bits, moved/error/fired results, used work, complete
poll strings, node point allocation identities and routing costs match. The
unmodified baseline differs in **24 cases**; the candidate-read fix alone leaves
**seven poll differences**, resolved by the live owner-box fix. Frozen-source
repetition is byte-identical. No fixtures, exclusions or lossy normalization are
added to the implementation.

The preceding compaction owner bridge remains installed: one shared moves guard,
real original endpoint owners, requested directions independent of fallback
orientation, and nil versus allocated-empty abductions. Previous exact sizeless
trial restoration and fixed-origin bounds remain installed. The historical
synthetic unbound-endpoint witness and its explicit replacement/rejection contract
remain in the preceding immutable checkpoint.

Current evidence: `.reference/tala-compaction-live-candidates-20261022/`, including
SOURCE-MAPPING.md, both negative matrices, frozen source/build bindings and NEXT.md.
The preceding `.reference/tala-compaction-bridge-20261022/` checkpoint is immutable.

## Historical verification (TALA compaction checkpoint)

- **1,679 ordinary / 1,708 diagnostic passes**,
  zero failures; **24 historical ignores unchanged**. All 17 original-method and
  three explicit map-order-controlled cohorts remain exact in both builds.
- **67,352 retained input/output bindings**, **1,544
  original snapshots** and **488 current source/fixture
  bindings** verify original hashes.
- The new **1,074 live-candidate controls** pass in both builds. Prior **12 real
  endpoint controls**, **1,092 canonical move controls / 2,184 invocations per
  build**, **40 entry controls / 80 invocations**, **32 candidate controls**, **eight
  source range-rollback controls / 12 native attempts**, **16 transition controls**
  and **96 sizeless shift controls** remain exact.
- Rebuilt driver: **7,548 matching snapshots / 204 final graphs**, 34 layouts at
  seeds **0/1/2/7/33/42**; ordinary and observed final outputs agree.
- Rebuilt CLI: **297 outcomes / 301 complete PNG boards**. Complete nonempty
  inventories and decoded pixels match. Missing/corrupt/truncated/timed-out
  output fails. Formatting passes. Three older non-TALA warm suites keep their
  unchanged hashes/build bindings; they were not rerun here. No new WASM or
  platform qualification is claimed.

## Remaining work

Qualify whole-compaction work/context failures and rollback, including guarded
selected-owner point-only snapshots and checked inner shift scoring. The new
mutation controls have two ordinary nodes, fixed membership, no labels/groups
and one point mutation; they do not establish arbitrary multiple mutations,
map/header/size/route changes, foreign/non-root ownership, point lifetimes,
arbitrary Go map order or all candidate inventories. Broader numeric/error and
language/API/render/export/native/WASM/platform PLAN gates remain open. This
checkpoint does not establish whole-compaction, TALA or D2 parity.

## Disk audit and retention

All ceilings pass. Whole checkout: **11.63 GiB allocated /
24.48 GiB apparent**. `.reference/`: **6.46 /
17.63 GiB**; `target/`: **2.92 /
4.43 GiB**. Volume free: **128.60 GiB**. Hidden
paths and all hard-linked apparent paths are included.

This investigation retired **505.14 MiB** of scoped
obsolete/redundant artifacts and recovered **732.39 MiB**
through verified Applesauce LZFSE compression. Net allocated checkout change
from the fresh start: **+10.67 MiB**; gross savings are not described as net
shrinkage. Deletion/replacement ledgers cover the idle Go action cache, redundant
captures/build variants and setup staging copy. The Go extension was virtual;
no physical trace-tree source was created or deleted. Six retained reference
executables are classified. Required input/output closure, source snapshots and
failure witnesses remain; no executable body enters source freezes. APFS clone
backups were removed after byte/mode/non-compression-xattr validation. No bulk
moved outside this repo.

### Lane I66: TALA incident-slice reads (2026-10-26, Linux VM)

A CPU-clock profile of `real_world/spyre_encoder` identified synchronization
in spatial-overlap spacing checks. The incident-edge walk now holds one backing
read guard across the callback-free walk, preserving traversal and nil handling.
Source mapping: pinned `internal/placement/optimizer_spatial_index.go`'s
`doesOverlap` and `internal/layoutgraph/placement_access.go`'s `DeltaTo`.
No arithmetic or FMA order changed.

E2e: **933 SVG + 11 errors**, delta **0 fixed / 0 regressed** across all 966
rows, including 308 TALA SVGs. TALA library: **1,685 passed, 27 ignored**.
Five-repeat VM benchmark: **33/33 selected engine/case rows** match Go's SVG
bytes on every repetition; TALA CPU ratio **1.47x → 1.44x**. The script selected
11 cases per engine on this VM. Concurrent builds and other-lane changes, plus
different debug-info settings, prevent isolated attribution. One-run Spyre
profiles remain in `.reference/codex-runs/iso66-profile-{before,after}.txt`;
atomic operations and node-slice reads remain prominent. TALA raster parity could
not be rerun: its reference corpus is not mirrored on this VM. Clean-host
performance and raster qualification remain outstanding.
