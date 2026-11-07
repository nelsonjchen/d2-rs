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

- **TALA placement (resolved):** native arm64 and freshly rebuilt WASM now
  match pinned Go on the retained `stable:25`/`stable:27`, transpose and
  seed 20261310 row 1193 witnesses, including `w50-tala-grid-reduced.d2`.
  The later label-position hierarchy witness also matches in WASM (7/7
  complete compile/render responses).
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

Fixed in the 0.1.1 round (2026-11-06), each with contract tests:

- **PNG:** right-to-left override controls (Go's bidi runs, controls without
  ink admitted to native shaping); short `cylinder`, `callout` and
  `c4-person` shapes (full-box canvas bounds, signed inner-box labels);
  `--scale=0.05` (route-label interpolation before mask rounding); malformed
  PNG icons, now rejected after Go's container and configuration probe.
- **Errors and text:** raw invalid style keywords in diagnostics (Go's `%s`);
  four LaTeX style/line-break root causes (fenced declarations, trailing
  binary operators, superscript rows, fraction denominators); nested
  sequence diagrams (error precedence, empty and populated inner sequences,
  outer actor roles after message reinjection); escaped-name dimension keys.
- **Fuzz layouts (seeds 2026110616-18, 1,000 jobs each per engine):** all 19
  minimized witnesses match: grid direction from the last primary key, root
  `near` bounds, Dagre fitting below non-square ancestors, ELK padding from
  original child sizes and resize-time label shifts, inline HTML in
  Markdown, TALA explicit container widths and filtered-glob placement. The
  fuzzer now compares complete output inventories (every board and
  manifest), which earlier runs did not.
- **TALA:** the 200-node nested work-limit board stops in
  `AutolayoutTransactions` as Go does. Seeds no longer queue for a routing
  slot: every seed routes at once, as in Go. Fan-out at 100/200 nodes went
  from 1.46x/1.66x Go wall to 0.77x/0.80x (x86, five interleaved runs,
  identical SVGs). Parallel edges are 0.76-0.78x Go CPU at 50-200 edges;
  nested/100 peak RSS fell to 0.63x Go.
- **Raster performance:** a shadowed container with 60 children, 240
  double-bordered shapes, 240 gradient fills and 240/480-item Markdown all
  beat Go in CPU and RSS (row-bounded shadows, bounds culling before stroke
  expansion, reused gradient rows, shared code atlases).
- **WASM performance (first measurement):** Node 22 on arm64 against the
  pinned `d2js`, the 12 perf-bench inputs on every engine: median wall 0.33x
  (Dagre), 0.33x (ELK), 0.38x (TALA); worst row 0.69x; all 36 SVGs identical.
- **Windows:** the default tier passes and all 944 applicable e2e boards
  match on native MSVC (Windows Server 2025) and under Wine (GNU target,
  against a Windows build of Go). Paths follow Go's `AbsPath`/`HumanPath` and
  `filepath.Rel`, file errors use Go's message text, and Go-matching FMA and
  rounding replace C-library results that differ on Windows. d2-rs and Go
  also agree on all 933 SVG boards with CRLF line endings. CI compiles the
  Windows tests without running them.

Open, with witnesses from the sweeps:

- **TALA routing memory:** with all seeds routing at once, each concurrent
  seed adds about 1.6x Go's routing memory (fan-out/200 peak RSS 1.05x Go,
  `tpmjs` 0.94x). Profile one seed's routing state and shrink it.
- **TALA error path:** a 400-node fan-out fails identically in both at
  `NodePlacement` (`Dimensions w:859, h:39517`), but d2-rs takes 0.36 s and Go
  0.04 s to get there.
- **Performance beyond corpus size** (d2-rs/Go): TALA long labels at 100
  nodes 1.26x wall (1.05x CPU), measured before the routing change;
  compiling an 80,000-edge board before the edge-limit error at about 1.4x
  wall. Shadows, opacity, 3D, nested fills and dark theme stay under Go but
  their ratios grow with size. The corpus itself has no row over Go.
- **Unreplayed witnesses:** the original four LaTeX probes and three fuzz
  singletons (`*: suspend` globs, a `package` shape four levels deep,
  Gujarati next to an emoji) were not retained; 60 `suspend` and 18
  `package` variants match. Replay them if a sweep finds them again.

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
