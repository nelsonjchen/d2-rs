# Accepted implementation plan: full D2 v0.9.0 in Rust

## Current goal (set 2026-10-22): finish the port, ending in byte-identical SVG

Done means every applicable upstream golden matches at every tier: error
parity, target hash, and normalized `sketch.exp.svg` bytes (today 11/11,
875/944 and 0/944). The SVG tier is reached through a faithful d2svg port,
milestoned so each step is measurable:

- **S1 Harness.** Render upstream `board.exp.json` with the Rust d2svg port and
  compare it to `sketch.exp.svg` (`scripts/d2svg-golden.py`). This isolates SVG
  parity from layout parity: ~950 exact input/output pairs.
- **S2 Exact compression.** Port Go's `compress/flate` encoder and zlib
  wrapper (`crates/d2-rs-goflate`). WOFF font tables are zlib streams, so
  embedded fonts are only byte-identical with Go's encoder.
- **S3 Fonts.** Port `lib/font` (UTF8CutFont subsetting, Sfnt2Woff) and the
  d2svg font corpus/trigger logic.
- **S4 Core d2svg.** Wrapper, background, theme CSS, rectangles and text,
  basic connections, markers and masks; `sanity/*` byte-identical.
- **S5 Coverage by cluster.** Remaining shapes, labels, icons, links and
  tooltips, class/table/code/Markdown/LaTeX, patterns and gradients, sketch,
  multiboard and animation, appendix. Fix the largest SVG-tier clusters first.
- **S6 Switch over.** (CLI SVG switched 2026-10-22; WASM SVG switched 2026-10-23; exports next.) The CLI/WASM SVG output and raster/PDF/PPTX inputs move
  to the d2svg port; the `d2-rs-*` dialect renderer is retired once nothing
  depends on it.

In parallel, close the remaining target-tier queue in `docs/STATUS.md`.

## Execution strategy (revised 2026-10-22)

The fixed contract below is unchanged. What changed is how work is chosen.
Through 2026-10-22 the Codex-driven effort drifted into ever-narrower TALA
mutation/rollback control matrices while no broad measurement existed of
where d2-rs stood (see `docs/SCOREBOARD-WORKFLOW.md` for the history, the
operating loop and its rules). From now on:

1. **Scoreboards are the primary gate.** Upstream ships exact goldens; score
   every one of them, not hand-picked fixtures:
   - e2e boards (`e2etests/testdata`, ~950 boards across dagre/elk/tala):
     `scripts/e2e-scoreboard.py`, with cases dumped from the pinned test
     package by the env-gated hook in `../d2-v0.9.0-trace/e2etests/e2e_test.go`.
     Tiers per board: error parity → target hash (`d2-<hash>` = D2's hash of
     shapes/connections/root/config) → normalized SVG bytes.
   - Next tiers to add, in order: d2compiler (375 goldens), d2oracle (445),
     d2ir (103), d2parser (45), d2exporter (19), d2sketch (39),
     e2etests-cli (61), then raster/PDF/PPTX via the existing raster gate.
   Every change must report the scoreboard delta and **zero regressions**
   (`scripts/e2e-scoreboard-delta.py`).
2. **Prioritize by boards fixed per root cause.** Cluster mismatches by field
   (`scripts/e2e-target-diff.py`) and fix the largest clusters first.
   Exporter/style bugs that fail identically under all three engines come
   before engine-specific geometry. Reduce each layout failure to a minimal
   witness (`scripts/reduce-d2-witness.py`) before source comparison.
3. **Investigations must name the boards they fix.** A trace/control matrix
   is justified only when it targets a scoreboard failure. Open-ended
   qualification of internal TALA states without a failing board is
   deprioritized; the retained TALA control corpora remain regression
   evidence, not the work queue.
4. **One target model.** Port d2exporter into a single target-diagram
   builder consumed by the hash, the SVG renderer and exports. Today the hash
   path re-derives styles separately from the renderer and has drifted
   (isCurve, theme special rules, dashed stroke, Markdown bold were all wrong
   only in the hash).
5. **Byte-identical SVG becomes the goal** (revising "equivalent SVG" below).
   d2-rs currently emits its own `d2-rs-*` SVG dialect. Porting d2svg
   faithfully turns ~950 goldens into exact checks, restores the
   user-facing CSS class contract (`fill-B6`, `stroke-B1`, themes, dark mode)
   and makes raster parity follow from SVG parity instead of being proven
   separately per case.

## Fixed contract

Build native libraries, the `d2-rs` executable and browser/Node WASM bindings.
All built-in algorithms execute in Rust without Go, a JS engine or a browser
runtime. Thin JS WASM bindings are allowed. Match exact language semantics,
diagnostics, geometry and visual output. SVG output targets byte identity
with d2svg (see the execution strategy); compression and incidental archive
metadata need not match bytes. Correctness
gates completion; performance is measured rather than presumed.

Pin D2 v0.9.0 at d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda and Go 1.27.0.
Update `../d2` to that detached commit, preserving master; keep it pristine.
Use the dedicated `../d2-v0.9.0-trace` worktree for reproducible
instrumentation. Store verified assets and dependencies in ignored
`.reference/`, with tracked versions,
checksums and acquisition commands. /tmp is only for disposable outputs. The
parity target is the original Go D2 v0.9.0 implementation, including bundled
TALA. Initialize local master; no remote publication, registry release or
website work. Track later upstream versions separately.

## Architecture

Use d2-rs-core (AST/IR/graph/target/geometry/options/errors), d2-rs-language
(parser/compiler/formatter/imports), d2-tala-rs, d2-rs-dagre, d2-rs-elk,
d2-rs-layout (selection/grid/sequence/near), d2-rs-text (fonts/measurement/
Markdown/code), d2-rs-math, d2-rs-sketch, d2-rs-render (scene/SVG/raster/ASCII/
animation), d2-rs-export (PNG/GIF/PDF/PPTX), d2-rs-editor, d2-rs-lib,
d2-rs-cli and d2-rs-wasm. Add packages when their implementation begins;
empty packages do not count as implemented subsystems.

Mirror upstream package subdivisions and recognizable functions with Rust
ownership and shared types. Expose parse/compile/layout/render/format/edit
separately. Preserve AST → IR → graph → target → scene distinctions. Provide
explicit filesystem, asset and cancellation interfaces across native/WASM.
Map upstream symbols/revisions, dependency licenses, Rust modules, tests and
remaining differences. Translate only dependency functionality reachable by D2.

TALA must follow upstream engine/layoutgraph/hierarchy/placement/placementcost/
routing/packing/labeling and supporting modules. Translate algorithms and data
structures from pinned D2 source, resolve disagreements from pristine source,
and replace any Go emulation that differs from v0.9.0.

## Feature checklist

- [ ] Parser: quoting, escapes, comments, block strings, nested maps, edges,
  indexed edges, partial parses, byte/Unicode/UTF-16 positions and diagnostics.
- [ ] IR/compiler: scopes, references, variables, substitutions, classes,
  inheritance, globs, recursive globs, filters and cycles.
- [ ] Imports: relative/selective/spread imports, VFS, dependency tracking/errors.
- [ ] Boards: layers/scenarios/steps, inheritance, targeting, cross-board links.
- [ ] Graph: containers/ports/tables/classes/styles/shapes/icons/labels/arrows.
- [ ] Formatter, validation and malformed-input behavior.
- [ ] Editing: create/set/delete/rename/move/reconnect/import updates.
- [ ] Editor helpers: completions, reference/import ranges, board queries;
  do not invent a separate LSP server beyond upstream's surface.
- [ ] TALA full stages, scoring, seeds/races, rollback, cancellation/work limits.
- [ ] Dagro plus required Graphlib; elk-go plus reachable dependencies.
- [ ] Grid/sequence/near, fixed dimensions/positions, nested layouts/descendants.
- [ ] Engine flags/features/precedence; default Dagre and format overrides.
- [x] External layout-plugin support is explicitly excluded; all target engines
  run in-process. Existing adapter scaffolding is not a compatibility gate.
- [ ] Fonts: bundled/custom metrics, fallback/shaping/wrapping/kerning/Unicode.
- [ ] Native SVG Markdown, lists/tables/links/styles; code syntax/measurement.
- [ ] MathJax TeX-to-SVG behavior/tables; font subsetting/embedding/licenses.
- [ ] Local/remote/embedded assets, caches, HTTP compression, EXIF/first frames.
- [ ] SVG shapes/edges/labels/themes/dark themes/legends/patterns/gradients/
  clipping/masks/filters/effects; rough sketch randomness/fills.
- [ ] Scene/SVG import, resource checks and unsupported errors.
- [ ] Raster geometry/strokes/text/images/compositing/color/animation.
- [ ] PNG; GIF frames/timing/palettes/loops; PDF/PPTX pages/slides/navigation/
  links/assets/text; animated SVG, ASCII and Unicode output.
- [ ] Multiboard output, appendices/tooltips/bundling/padding/scale/backgrounds.
- [ ] CLI files/stdin/stdout/formats/exits; layout/themes/fmt/validate/play/version.
- [ ] Flag/env/config precedence including D2_*; watch/import watching/live
  preview/reload/browser controls; timeout/cancellation/partial output.
- [ ] Native public APIs/errors; WASM compile/render/raw helpers including
  deprecated-but-present exports; workers/Node/types/disposal/pending errors.
- [ ] Preserve platform boundaries rather than requiring browser subprocesses.

## Verification

Import unit tests, inline Go fixtures, txtar, integration/dependency tests and
fuzz regressions, not just .d2 files. Record fixture provenance. Expand
seeds/permutations/nesting/malformed/resource cases. Track every test as
passing/failing/upstream-skipped/inapplicable with reasons; unimplemented is
never passing. Revalidate historical claims: old benchmark binary was not the
official asset; old snapshot comparator does not prove detailed trace parity.

Compare each stage: AST/ranges → IR → graph → measurement → layout → target/
scene → exports. Detailed traces include RNG stream/operation/draw count,
actual map/sort traversal, candidate/score/decision, transaction and routing
events. Preserve exact float bits, stable entity identity, meaningful event
order and coordinates. Omit timing/addresses/thread identity only. Fix the
earliest divergence, minimize witnesses, inspect surrounding functions, use
debugger watchpoints, source hooks, readable SVG diffs and disassembly where
needed. Verify hooks do not affect ordinary results. Test individual seeds and
races separately; explicitly report genuine upstream nondeterminism.

Exact semantics/geometry/seeds/deterministic errors are required. Compare
equivalent SVG and exact pixels under a fixed renderer/fonts/assets. Compare
decoded PNG, GIF frames/timing, PDF/PPTX content/navigation/rendered pages.
Normalize only documented encoding/metadata differences. Cancellation and
work limits must match semantics, not exact elapsed instants.

Benchmarks compare hash-verified official D2 and native Rust using equal
options/seeds. Record artifacts/toolchains/hardware/
threads/warmups/distributions/peak memory. Isolate startup/parser/measurement/
layout/render/encoding and direct-engine timing. Optimize measured bottlenecks
and rerun parity. Normal/release builds compile out trace argument construction;
explicit diagnostics use an optimized profile with symbols.

## Ordered milestones

1. References, fresh workspace, MPL attribution, manifest, source/test inventory,
   corrected performance baseline.
2. Shared model and upstream-shaped TALA, final-output and detailed trace gates.
3. Rust-only D2 parse/compile/measure/TALA/SVG vertical slice (not completion).
4. Full language/boards/shared layouts/Dagre/ELK/editor functionality.
5. Full text/math/sketch/SVG/raster/ASCII/animation/exports.
6. CLI/watch/native/WASM/JS interfaces (external layout plugins excluded).
7. Full ledger, fuzz/determinism/platform/package/performance qualification.

CI targets Linux/macOS/Windows ARM64/x86-64 and browser/Node WASM. Use uv and
pnpm. Develop and test with the latest stable Rust, pinned explicitly in
rust-toolchain.toml and CI for reproducibility (currently 1.98.1). The minimum
Rust version matches that pin. Update the pin, manifest and CI together for
stable upgrades.
Package MPL/notices/corresponding-source instructions with every artifact.
Completion requires all applicable capabilities and parity gates, with upstream
limitations recorded. Do not infer full compatibility from a partial corpus.
