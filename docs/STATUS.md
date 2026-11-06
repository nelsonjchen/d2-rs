# D2 parity status

The compatibility target is the Go D2 commit pinned in
[reference-manifest.json](../reference-manifest.json). Go and JS are test
oracles; the implementation runs in Rust. Oracle artifacts are regenerated
into the ignored `oracle-cache/` as described in [FIXTURES.md](FIXTURES.md).

## Oracle fixtures from recipes

The default and oracle test tiers were last qualified with zero failures on
arm64 and x86-64, each against fixtures built for the same architecture. The
original oracle drift, including the mathjax-go update and mixed PNG aspect
rounding, was resolved. Individual runs can still fail or time out and must
be checked before claiming a current pass.

## Open drift

The differential harness renders upstream inputs and mutations of them with
the pinned Go CLI and d2-rs under the same options. All 5,024 original jobs
matched at the last qualification. Expanded seeded sweeps after the export
rework (arm64 seeds 20261103-20261107, amd64 seeds 20261104-20261266, about
4,360 jobs each) and the WASM public API differential (2,176 of 2,181 jobs)
fixed every cluster below except the open ones. Each seed's sweep matches all
but a handful of jobs. Open clusters, with witnesses in
`oracle-cache/reports/differential/` and `wasm-differential/`:

- **TALA placement:** the nested-hierarchy, SQL-table and deep-nest
  witnesses now match on amd64; WASM `stable:25`/`stable:27` remain open,
  as does WASM seed 20261310 row 1193, which reduces to an ordinary nested
  hierarchy without grid properties (`w50-tala-grid-reduced.d2`): a root
  leaf receives container dimensions.
- **Raster pixels:** the recorded simulator PNG, color-emoji GIF, arrowhead
  PNG and markdown cylinder PNG witnesses now match on amd64 and arm64.
- **Go panics:** five inputs per seed crash the pinned Go itself (no oracle
  output); d2-rs reports an error instead. Recovered Go panic messages carry
  Go runtime stacks in the WASM API; the fault descriptions agree, but the
  stacks include Go addresses and goroutine state.

## Beyond the upstream corpus

The upstream corpus samples a narrow slice of D2: few code languages, small
diagrams, well-formed input. Passing it hid real gaps, so these sweeps run
d2-rs and the pinned Go CLI on generated inputs (outputs and witnesses go to
`oracle-cache/reports/`):

- `scripts/scale-sweep.py`: layout families from 25 to 800 nodes on every
  engine, plus Dagre PNG and PDF. A row fails when it is over Go or when its
  ratio to Go grows with size.
- `scripts/raster-stress.py` and `scripts/feature-stress.py`: one raster or
  language feature at a time at growing size (shadows, patterns, gradients,
  opacity, sketch, Markdown, code, emoji, icons, globs, classes, vars,
  imports, many boards, PPTX/PDF, parallel edges, LaTeX, themes, extreme
  sizes and scales).
- `scripts/registry-sweep.py`: every shape, label position, style keyword,
  arrowhead and CLI flag, plus sequence diagrams, grids, SQL/class shapes,
  Markdown elements, near constants and icons, as small boards.
- `scripts/chroma-differential.py`: one snippet per Chroma lexer (368 cases).
- `scripts/fuzz-differential.py` and `scripts/fuzz-minimize.py`: random
  structural mutations of corpus inputs, flagging Rust panics, hangs and
  exit, error-text or output divergence, then line/character minimization.

Fixed through these sweeps: code blocks in every Chroma language (only nine
were translated), ELK network-simplex order and subtree pruning on large
graphs, Dagre labels on short shapes, Dagre horizontal layouts with a
disconnected container, Markdown list lazy continuation, LaTeX line breaks
after operators and many MathJax cases, sequence messages to undeclared
nested actors, the import-spread substitution budget, a Dagre timeout on a
densely cross-linked 500-node graph, TALA's 50,000-edge input limit (an
80,000-edge board ran out of memory instead of failing as Go does), and
per-band or per-frame raster
rework (sequence PNG 9.7x to 0.68x Go CPU at 50 actors, emoji 3.2x to 0.7x,
repeated icons 1.9x CPU and 4.4x RSS to 0.7x and 1.0x, animated GIF, code
blocks, gradients, opacity, nested Dagre).

Open, with witnesses from the sweeps:

- ~~**Raster text:** bidi override controls (U+202E/U+202C) differed in PNG.~~
  Fixed Go-compatible bidi runs and native admission of controls without ink;
  the complete hard-text board now matches Go byte for byte on arm64.
- ~~**Short shapes in PNG:** `cylinder`, `callout` and `c4-person` with an
  explicit height of 25-31 px differ from Go in PNG; their SVG matches.~~
  Fixed full-box canvas bounds and signed inner-box label placement.
- ~~**Small scale:** `--scale=0.05` differed in PNG (100 and 500 nodes).~~
  Fixed arm64 route-label interpolation before mask-box rounding; both match
  Go byte for byte.
- ~~**Malformed images:** a malformed PNG icon that Go rejects
  (`imageasset: probe ... malformed`) renders in d2-rs.~~ PNG admission now
  validates the complete container and configuration before retention.
- **Error text:** a right-to-left override character in an invalid keyword is
  escaped (`‮`) in d2-rs errors and raw in Go's.
- **Markdown:** a tab-indented empty heading inside a block differs.
- **LaTeX:** four probes where a style declaration spans a line break.
- **TALA work limits:** a 200-node nested board exceeds TALA's work limit in
  both, but Go stops in `AutolayoutTransactions` and d2-rs in
  `GapNormalizationTransactions`, so the error text differs.
- **Fuzz singletons:** duplicated `grid-rows`, `*: suspend` globs, a `package`
  shape four levels deep, Gujarati next to an emoji; one witness each.
- ~~**Non-TALA raster performance:** shadowed containers, double borders,
  gradient fills and Markdown exceeded Go at large sizes.~~ Regional shadow
  inputs, bounded stroke caches, shared font-size atlases and row gradient
  sampling bring all four below Go locally on arm64, with identical PNG
  bytes; quiet-host confirmation is pending.
- **Performance beyond corpus size** (quiet arm64, d2-rs/Go): TALA
  with 100 parallel edges at 1.24x CPU and growing; TALA with 100 classes at
  1.10x RSS; TALA at scale (`scale-sweep.py`): nested at 100 nodes 2.24x
  RSS, long labels at 100 nodes 1.15x RSS, fan-out at 200-400 nodes 1.5-1.75x
  wall with CPU near Go; compiling an 80,000-edge board before the engine's edge-limit
  error at about 1.4x Go wall. Shadows, opacity, 3D, nested fills and dark theme stay under Go
  but their ratios grow with size. The corpus itself has no row over Go.
- **Long fuzz:** seeds 2026110616/17/18, 1,000 jobs each on Dagre/ELK/TALA,
  find 6/6/7 differences. Complete output inventories and all 22 matching
  empty-output outcomes are checked; no crash, hang, timeout or invalid SVG.
  Minimized front-end/Markdown/layout witnesses cover root `near`, repeated
  grid dimensions, circle containers/globs, `0` versus `00` dimensions,
  inline HTML measurement and diagnostic text/precedence. Three TALA-only
  witnesses cover explicit container width fitting and filtered-glob placement.
  Witnesses and owner handoffs: `oracle-cache/reports/x16/handoff.json`.
  X18 fixes all nine assigned Dagre/ELK output witnesses: HTML self-closing
  and leading-whitespace measurement, final grid-key order, root-near bounds,
  Dagre fitting below non-square containers, and ELK child-size estimates and
  resized-label margins.
  The shared grid/near causes also cover TALA jobs 740/203; diagnostics and
  the three TALA-only placement witnesses remain with Q1/T2.
- **WASM performance:** Node 22.21.1 on arm64, pinned d2js versus current Rust
  WASM, the 12 perf-bench inputs on every engine, three interleaved samples
  after warmup. All 36 SVGs match byte for byte. Median wall Rust/Go ratios:
  Dagre 0.33x, ELK 0.33x, TALA 0.38x; worst row 0.69x. Process CPU samples are
  retained separately in `oracle-cache/reports/x16/wasm-performance.json`.
  Two early-call CPU excesses disappear with additional warmup: Dagre
  arrowhead scaling is 0.32x Go after three warmups, and TALA grid across cells
  is 0.45x after ten (interleaved samples, 500 ms settling, identical SVGs).
- **Windows:** the GNU target passes the default tier under Wine 9 and matches
  Windows Go on all 944 applicable e2e boards, including TALA. CRT FMA and near-halfway
  rounding are bypassed in the affected math paths. Absolute forward-slash
  source paths now retain their import base across drives; backslash source
  paths resolve imports against the working directory in pinned Go too.
  Diagnostics follow Windows separators, case folding and OS error text.
  Native MSVC remains unqualified; CI still compiles its tests without running.

Go progress messages that differ only under concurrent load (identical output
hashes, not reproducible when Go runs alone) are harness noise, not drift.
Historical run details are in the lane reports and d2-rs-history.

## Verification

Run `cargo test --locked --workspace` for the self-contained default tier and
`scripts/test-oracle.sh` for the oracle tier. The e2e, differential and
performance scoreboards and their prerequisites are documented in
[SCOREBOARD-WORKFLOW.md](SCOREBOARD-WORKFLOW.md). The last qualified e2e
baseline was 944 of 944 applicable boards matching (933 SVGs and 11 expected
errors). [BENCHMARKS.md](BENCHMARKS.md) describes the performance baseline.
