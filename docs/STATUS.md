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
- **TALA nested placement (resolved):** D4's six-container witness now
  matches Go's 1,748 x 1,660 SVG. Accepted cluster rotations publish their
  geometry before the retained owner projections are refreshed for mirroring.
  Wrapper depths one through six and seeds one through three are covered by
  `oracle-cache/reports/T32/replay-candidate.json` and a CLI padding contract.
- **TALA nested-edge placement (resolved):** D43's
  `n; id->x->y.z; id->x.d->y` now matches Go. Hierarchy recognition keeps
  the original abducted source/sink endpoints when testing the three-rank
  exclusion. Root/nested scopes, reversed flow and shared/distinct endpoint
  variants match in `oracle-cache/reports/T33/replay-candidate.json`.
  D44's nested-grid-branches witness also matches, with default seeds and
  explicit seeds one through three (`oracle-cache/reports/T35/handoffs.json`).
- **TALA ancestor-edge clustering (resolved):** D46 job 47 and D47 jobs
  372/891 now match Go. Cluster discovery rejects members connected to their
  ancestors before resizing; child scopes ignore the detached edge incidences.
  Seeds one through three and nested variants are replayed in
  `oracle-cache/reports/T36/handoff-final.json`.
- **TALA nested ancestor routing (resolved):** route reconstruction follows
  predecessors through the source center until nil or a cycle, preserving
  Go's exact OVG path during publication. D46 job 47 now matches at wrapper
  depths zero through five, in both edge directions, with default seeds and
  explicit seeds one through three (`oracle-cache/reports/T37/variants.json`).
- **TALA cloud placement/routing (resolved):** D46 job 664 now matches Go
  with default seeds and explicit seeds one through three. Rejected transpose
  trials restore cluster arrangement, desired arrangement, padding and vessel
  dimensions before the next trial or alignment pass. The retained ancestor
  matrix improves from 40/44 to 44/44 matches; cloud, label, edge-direction,
  shape and enclosing-container variants improve from 4/84 to 76/84, with no
  lost matches. The eight enclosing-wrapper rectangle variants now match too:
  nested flips preserve Go's check for saved ancestor spacing overlaps becoming
  exact. All 84 retained variants match, up from 76, with no lost matches
  (`oracle-cache/reports/T328/{cloud-replay,cloud-candidate}.json`).
- **TALA circle-container alignment (resolved):** D48 job 402 now matches
  Go with seeds one through three. Container-alignment scoring excludes
  temporary cluster vessels represented by a container member's arena slot.
  Explicit children, reversed edges and enclosing-wrapper variants match too.
  Recursive member fits now resize the retained vessel node as well as its
  cached size before later components compute their placement cell size.
  All 28 retained cases match, up from 20, with no lost matches
  (`oracle-cache/reports/T329/circle-candidate.json`).
- **TALA route-search errors (resolved):** D48 job 236 now matches Go's
  work-limit error with default seeds and seeds one through three. Workers
  preserve the guard failure before deriving an empty-route invariant, and
  deferred cleanup preserves the original cause without another poll. D49
  jobs 45/476 already match (`oracle-cache/reports/T326/witness.json`).
- **TALA hierarchy traversal (resolved):** D413 jobs 198/686 now match Go.
  Edges promoted from descendants follow the ancestor's existing incidences
  before component traversal and the seeded hierarchy shuffle. Edge direction,
  styles, declaration order and two enclosing-wrapper depths improve from 0/60
  to 60/60 matches; both witnesses have CLI regression contracts for all three
  seeds (`oracle-cache/reports/T327/{variants,contract-686-comparison}.json`).
  D413 job 615, D414 job 465 and the D419 expanded witness already match.
- **TALA NaN-height error:** D43's complex nested board reports a NaN
  object height in Go and a route-search work-limit error in Rust (the
  guard error previously became an empty-route invariant). A 77-byte nested
  sequence/grid reduction still fails Go's `RouteEdges` input validation,
  while Rust renders it. Go has nonfinite sequence span bounds before routing;
  compare nested extraction/reinjection and span sizing in `d2-rs-layout`
  before changing TALA. D4 handoff and complete outcomes:
  `oracle-cache/reports/T34/{nan-exit-minimized.d2,nan-height.json}`.
- **Repeated TALA seed flags:** Go appends repeated `--tala-seeds` values;
  Rust replaces the earlier list, which can select a different layout.
  This CLI issue predates the nested-placement fix. Witness and comparison:
  `oracle-cache/reports/T32/nested-minimized.d2` and
  `repeated-seed-flag-candidate.json` (flags `--tala-seeds=1 --tala-seeds=2`).
- **Raster pixels:** the recorded simulator PNG, color-emoji GIF, arrowhead
  PNG and markdown cylinder PNG witnesses now match on amd64 and arm64.
- **Go panics:** five inputs per seed crash the pinned Go itself (no oracle
  output); d2-rs reports an error instead. Recovered Go panic messages carry
  Go runtime stacks in the WASM API; the fault descriptions agree, but the
  stacks include Go addresses and goroutine state.
- **Fresh fuzz findings (D41):** Dagre horizontal compound cross-axis
  spacing now matches (D45): retain the border model's external-node lanes
  after cross-rank spacing. The reversed-flow self-loop, constant-`near`
  sequence lifeline ordering and malformed LaTeX `\end{` findings are fixed.
  Seeds 2026110741–43 produced four findings in 3,000 jobs. Minimized
  inputs are in `oracle-cache/reports/d41/witnesses/`.
- **Fresh fuzz findings (D42):** SQL-table self-loop column ports now match
  pinned Go's `d2graph.addSQLTableColumnIndices` early return. The sketch
  root-border PNG/PDF findings are fixed by preserving exact dash operands.
  Minimized witnesses and options are in `oracle-cache/reports/d42/witnesses/`.
- **Fresh fuzz findings (D43):** seeds 2026110753/55/57 (1,000 jobs per
  engine) found the known Dagre compound-spacing cluster, an ELK nested-grid
  edge-clipping mismatch, and TALA nested-edge placement/error-path differences.
  The ELK finding is fixed (D44): grid edges now use reinjected label/icon
  state rather than recomputing defaults from the original measured boxes.
  Reduced witnesses are in `oracle-cache/reports/d43/witnesses/`; TALA
  handoffs are `witness-D4-tala-{nested-edge-placement,nan-height}.d2`.
  Two other inputs crash pinned Go. The original ELK seed now has no SVG diff.
- **Fresh fuzz findings (D44):** feature-check diagnostics now preserve raw
  object IDs, and structured grid headers use the container-level font before
  the header adjustment. The Dagre compound-spacing cluster is fixed (D45); the new
  TALA-only nested-grid branch witness is
  `witness-D4-tala-nested-grid-branches.d2` in the lane reports directory.
- **TALA nested placement:** expanded seed 2026110745 differs on
  `fixed:tala-nested-sequence-placement` (theme 102, dark theme 200,
  pad 20, scale 1, center). Witness and exact options are retained in the
  same D41 directory for Lane T3; D419 reconfirmed it unchanged and retained
  `witness-D4-tala-D419-expanded.d2` in the lane reports directory.
- **Fresh fuzz findings (D45):** seeds 2026110769/71/73 found suspended
  endpoint label styles, Unicode capitalization, zero-sized grid children
  and containers, ELK `multiple: t`, and nested empty Markdown list drift.
  The Markdown list drift is fixed (D412): a list item's opening content
  can immediately open a child list, matching Goldmark's block descent;
  zero-sized Markdown viewports also retain Go's empty raster paint.
  Suspended endpoint styles and ELK label placement are fixed (D46):
  recreated objects retain their label text and completed-hierarchy defaults,
  including regular sequence-actor text.
  Seven findings are minimized in `oracle-cache/reports/d45/witnesses/`;
  the TALA-only fan-in witness is handed to T3 as
  `witness-D4-tala-fan-in-empty-endpoints.d2` in the lane reports directory.
- **Fresh fuzz findings (D46):** seeds 2026110777/79/81 produced six
  findings in 3,000 jobs. Two repeat the nested empty Markdown list drift
  fixed in D412;
  the empty-first-step root output path is fixed (D47): CLI dispatch counts
  folder-only children, matching Go's `output/index.svg`. Reductions are under
  `oracle-cache/reports/d46/witnesses/`. Three TALA findings reduce to two
  TALA-only clusters, handed to T3 as `witness-D4-tala-D46-{47,664}.d2`
  in the lane reports directory.
- **Fresh fuzz findings (D47):** seeds 2026110785/87/89 found four output
  differences in 3,000 jobs, plus one pinned-Go panic. Two shared-code
  reductions: the multiboard recursive hash is fixed (D48), retaining mapless
  sibling boards in traversal and target selection; sequence/grid sizing
  beside a disconnected container remains open. Reductions and
  cross-engine replays are in `oracle-cache/reports/d47/`. Two TALA-only
  container-edge witnesses are handed to T3 as
  `witness-D4-tala-D47-{372,891}.d2` in the lane reports directory.
- **Fresh fuzz findings (D48):** seeds 2026110793/95/97 found three
  mismatches in 3,000 jobs, plus one input that panics in pinned Go and Rust.
  The shared sequence target-hash finding is fixed (D49): actor references
  retain Go's child-before-parent compilation order before the AST sort.
  Reductions are under
  `oracle-cache/reports/d48/witnesses/`. TALA-only routing-error and tree
  placement handoffs are `witness-D4-tala-D48-{236,402}.d2` in the lane
  reports directory; the routing-error reduction reached its 180-second budget.
- **Fresh fuzz findings (D49):** seeds 2026110801/03/05 found three
  mismatches in 3,000 jobs. Markdown `<->` now renders correctly (D410):
  HTML block tag names require an initial ASCII letter, matching goldmark.
  Two TALA container-edge outputs differ (handoffs
  `witness-D4-tala-D49-{45,476}.d2`). Expanded seed 2026110807 exposes
  small-scale multiple/double-border raster drift is fixed (D411): the
  native painter now retains both borders on the back copy and Go's pattern
  order. The sketch animated-edge drift is fixed (D418): time-zero raster
  frames retain the scene's exact reverse dash phase instead of the public
  SVG's six-decimal value. Reduced witnesses and exact options are under
  `oracle-cache/reports/d49/`.
- **D410 search:** fresh Dagre/ELK/TALA seeds 2026110809/11/13 match all
  3,000 SVG jobs. Expanded seed 2026110815 matches 4,408 jobs; its only
  mismatch is the retained TALA nested-sequence handoff (five Go failures).

- **Fresh fuzz findings (D411):** the Dagre seed 2026110817 sequence-actor
  ordering finding is fixed: actors now sort by their earliest reference
  outside a glob before placement and lifeline creation, matching Go. The
  32-byte reduction and original source are under `oracle-cache/reports/d411/`.
  Seeds 2026110817/19/21 now match all 3,000 SVG jobs (one finding before
  the fix); no panic, signal or hang findings. Expanded seed 2026110823
  matches 4,405 jobs, with only the existing TALA nested-sequence handoff
  and five pinned-Go failures.

- **Fresh fuzz findings (D413):** seeds 2026110833/35/37 found five
  differences in 3,000 SVG jobs. The Markdown hang is fixed: fallback sizing
  now uses Goldmark's ATX heading rule for both heading recognition and
  paragraph termination. Native Markdown validation also precedes sizing, so
  raster/text exports report Go's error before layout. The 13-byte reduction
  is under `oracle-cache/reports/d413/witnesses/`. Its initial same-seed replay
  had 2,996 matches and four retained differences, with no Rust hang or panic.
  D414 fixed the fourth: applied classes now recursively validate invalid child
  maps before the class-field diagnostic, while keeping `beginClass` cycle
  protection active for the full traversal. Three TALA-only reductions remain as
  `witness-D4-tala-D413-{198,615,686}.d2` in the lane reports directory.
  D417 fixes D412's three small-scale Unicode PNG rows: native COLRv1 text now
  reuses the retained f64 canvas transform instead of rebuilding it from f32
  renderer state. The family-emoji reduction and all 37 one-level pixels in
  the original now match pinned Go across Dagre, ELK, and TALA.
- **Fresh fuzz findings (D414):** Dagre seed 2026110841 matched all 1,000
  jobs. ELK seed 2026110843 found one compound self-loop failure: hierarchy
  preprocessing now preserves the default output/input port sides, its
  positional refresh no longer reruns free-loop side assignment, and the
  model-order port comparator reverses a side only when both ports have
  incoming edges, matching pinned elk-go. The same ELK seed now has 990
  rendered and 10 empty-output matches with no findings. TALA seed
  2026110845 has 999 matches and one TALA-only container fan-in difference;
  its reduction is `witness-D4-tala-D414-465.d2` in the lane reports directory.
- **Fresh fuzz findings (D415):** Dagre/ELK/TALA seeds 2026110849/51/53
  produced one difference in 3,000 SVG jobs. The ELK nested sequence/grid
  finding is fixed: crossings removed with an extracted grid actor no longer
  consume a parent sequence message row before reinjection, matching
  `d2layouts.LayoutNested`. The minimized witness is under
  `oracle-cache/reports/d415/witnesses/`. Expanded seed 2026110855 retains
  three D412 small-scale Unicode PNG rows and the known TALA nested-sequence
  row; the other 4,212 jobs match and five inputs fail in pinned Go.
- **Fresh fuzz findings (D418/D419):** Dagre/ELK/TALA seeds
  2026110857/59/61 produced two differences in 3,000 SVG jobs. Both are fixed:
  the Dagre-only ingress-lane correction no longer collapses outgoing
  child-to-external lanes, and d2grid accepts zero-sized cells as Go does.
  The exact D418 Dagre and ELK seeds and fresh D419 seeds 2026110863/65/67
  now match all 5,000 jobs. Reductions are under
  `oracle-cache/reports/d418/witnesses/`.

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
  The differential fuzzer accepts SVG, PNG, PDF, PPTX, GIF and TXT plus exact
  render flags, and includes upstream `.d2` files and raw Go-test scripts.
  A fresh 2026-11-08 sample matched all 250 selected format/flag jobs; PDF
  runs require the repository's clock-injected pinned oracle. A further
  1,000-job Dagre PNG run (seed 20261108241, sketch/theme/scale/center flags)
  found ten raster differences. Default-ignorable ZWJ shaping is fixed; its
  exact replay exposed a multiline class-header mismatch: private raster
  geometry now keeps Go's `labelHeight + 10` floor instead of the public
  sketch SVG's two-row height. Unset Markdown label positions now retain
  `d2target.Diagram.BoundingBox`'s top-left canvas bounds independently of
  the centered Markdown painter. Translated sketch-shadow patterns are also
  fixed: filter crops retain the prepared unshifted f64 paint matrix instead
  of reconstructing it after local translation. Native sketch ellipses now
  recover Go's four f64 cubic quadrants from usvg's eight-cubic lowering
  before flattening. Root PNG primitives now restart their scanline active
  lists for each 256-row destination just like `RenderBands`, preserving f32
  winding order after expired-edge swaps. Structured class and SQL-table
  streak rectangles now retain their f64 scene operands through SVG lowering,
  closing the final `grid_oom` fractional-coverage difference. The exact
  replay now has 993 rendered and seven empty-output matches with no findings;
  its summary is under `oracle-cache/reports/d429/`.

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
  seed still adds more routing memory than Go. Frozen OVGs now retain compact
  port-identity lists; flavors share immutable box, obstacle and port tables.
  Temporary vessel-port changes copy the port table only when needed. The
  per-hop index, container and flags now occupy eight bytes instead of 16.
  A fan-out/200 census finds 15,320 records per seed: 122,560 fewer retained
  payload bytes per seed. Nine interleaved arm64 repeats lower default CPU
  0.87% and wall 1.16%, with identical SVGs; peak RSS rises 0.65%, so no
  peak-memory gain is established. Additional-seed RSS remains
  above Go (about 1.44x in this repeat's
  separate-process peak estimate, which does not isolate live per-seed heaps)
  (`oracle-cache/reports/T316/bench.json`). T39's giant-Markdown
  control increased CPU 4.6% and wall 3.8%; routing was below 0.2% of its CPU
  samples. The supervisor's quiet-host gate and the remaining per-seed state
  stay open.
- **TALA error path:** a 400-node fan-out fails identically in both at
  `NodePlacement` (`Dimensions w:859, h:39517`). After removing whole-tree
  searches, reuse each parent's derived degree within an immutable fringe
  batch. Across 21 interleaved arm64 repeats, CPU falls another 3.28% and
  wall 2.48% (51.39 to 50.12 ms, Go 51.99 ms; 0.964x Go). Fanout/200 CPU
  stays unchanged, and its SVG is identical. Quiet-host confirmation remains
  (`oracle-cache/reports/T324/bench.json`).
- **TALA oversized inputs:** borrow the dispatcher's measured graph when
  padding and traversal order already match TALA, avoiding a full edge-metadata
  copy before validation. Five interleaved arm64 repeats on 80,000 edges lower
  peak RSS 478 to 405 MB (-15.2%), still 1.19x Go; CPU/wall change +0.48%.
  The edge-limit error is identical. Compiler IR and measured edge metadata
  still coexist before TALA entry; their contribution needs attribution
  (`oracle-cache/reports/T317/bench.json`).
- **Performance beyond corpus size** (d2-rs/Go): TALA long labels at 100
  nodes measured 1.046x wall before the fringe-degree change (prior CPU
  0.90x). Three interleaved arm64 phase captures use pinned Go's opt-in
  stage timing and temporary Rust timers,
  without enabling diagnostic traces; all complete SVGs match. Critical seed 1
  wall seconds, Rust/Go: preprocessing 0.00167/0.00329; `NodePlacement`
  38.409/34.760; `SwapStuff` 0.204/2.118; all later stages 0.409/0.422.
  `NodePlacement` for seeds 2 and 3 is 28.876/28.984 and 33.147/31.400 s.
  End-to-end wall is 39.046/37.336 s. The earliest slower phase is seed 1's
  `NodePlacement`; its 3.65 s excess is partly offset by faster swapping.
  Every stage and repeat is recorded in
  `oracle-cache/reports/T324/{phases,phase-summary}.json`; timing edits were
  removed. Further long-label changes are paused. T322/T323's spacing and
  sparse-publication candidates raised fanout/200 CPU 17-19% and were rejected.
  Shadows, opacity, 3D, nested fills and dark
  theme stay under Go but their ratios grow with size. The corpus itself has
  no row over Go.
  nodes 1.26x wall (1.05x CPU), measured before the routing change;
  compiling an 80,000-edge board before the edge-limit error at about 1.4x
  wall. Releasing the full compiler IR before layout reduced that witness's
  peak RSS from about 467 to 396 MiB, but made it about 10% slower because
  eagerly dropping the arena walks every allocation; that candidate was
  rejected pending a lifetime design without the destructor cost. Shadows,
  opacity, 3D, nested fills and dark theme stay under Go but
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
