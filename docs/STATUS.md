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

- **ELK hunt (resolved):** nested-container crossings and parallel labeled
  edges beside a container self-loop now match pinned Go's complete SVGs in
  all four directions. Model-order sorting honors saved long-edge target
  ports; detached loops no longer affect hierarchy influence or boundary
  layer constraints. Flow-axis publication retains unpadded points and Go's
  coordinate-conversion and mirroring order. Three CLI contracts cover 12
  complete SVGs; the default/oracle tiers and all 944 applicable e2e boards pass.
- **ELK small-graph enumeration:** 3,136 canonical two/three-node cases and
  1,024 seeded four/five-node cases (down/right, up to four edges) improve
  from 3,895 matches / 265 differences through 3,919 / 241 and 4,034 / 126
  to 4,156 / 4 and 4,160 / 0,
  with no lost match. Fixed and hierarchical ports now keep their order
  while ordinary edges are detached, preserving RoutingDirector's penalties;
  `n0: {n1}; n0 -> n0; n0.n1 -> n0` matches in all four directions.
  Self-loop ports now retain all transferred external-dummy coordinates;
  a redundant adjustment had moved them beside the reused hierarchy port
  again. Both the peer and enclosing-container witnesses match in all four
  directions. Long-edge splitting now follows Go's node-port adjacency,
  preserving dummy allocation order before P3; the nested placement and
  canvas-size case 03787 matches in all four directions too. The final four
  loop-path differences are resolved: node-local bends retain the unpadded
  node coordinate before adding the combined graph offset and padding.
  Mirroring preserves that cross-axis result. Three witnesses now match in
  all four directions, and the exact enumeration replay has no difference,
  Go failure or lost match. Expanded enumeration covers 115,782 canonical
  four-node cases through four edges and 59,984 five-node cases through two
  edges, down/right, with at most two containers and three children per
  container. It improves from 175,744 matches / 22 differences to 175,766 / 0,
  with no lost match or Go failure; all 4,160 earlier cases still match.
  Compound bounds now measure the content before translating geometry by
  padding, matching ComponentsCompactor and GraphActualSize arithmetic;
  a one-ULP excess had enlarged the container and canvas by one pixel.
  A CLI contract covers the reduced board, a self-loop and an enclosing
  container in all four directions. A seeded labeled/parallel-edge extension
  covers another 24,000 four/five-node cases and improves from 23,930 matches
  / 70 differences to 23,995 / 5, with no lost match or Go failure. All
  179,926 earlier enumeration outputs remain unchanged. Compound
  dummy creation now retains imported port order until PortRestorer runs;
  early free-loop restoration had reversed multiple-loop source ports.
  Labeled, unlabeled and enclosed triple-loop contracts cover all directions.
  The labeled extension now matches 23,996 / 24,000 cases, with no lost match.
  Case 12416's placement is fixed: crossing counts retain GraphInfoHolder's
  initial child-sweep membership after in-layer dependencies change order.
  The reduced board and longer-label variant match in all four directions.
  Ancestor edges now retain GraphTransformer's flow-axis frame through
  compound joining and Transferrer.edgeLayoutOffsets, including the ancestor
  source's base-offset subtraction. This fixes cases 12839, 18277 and 20619
  and the wrapped 12416 up/left route shifts; the labeled extension now
  matches 23,999 / 24,000 cases with no lost match. All 32 wrapper, direction
  and reversed-edge variants match, up from 26. Two CLI contracts cover
  24 complete SVGs. Case 21573's root-loop bends and labels now retain
  OrthogonalSelfLoopRouter's node-local slot arithmetic before adding the
  node position in PostprocessSelfLoops. The labeled extension matches all
  24,000 cases with no lost match or Go failure; all 179,926 earlier outputs
  remain unchanged. A CLI contract covers the original, longer-label and
  enclosed boards in all four directions.
  The five-node extension matches all 423,200 three-edge cases and the first
  44,896 four-edge cases in down/right flow, compared in 64-board batches
  with complete SVG inventories and manifests. Another 47,500 standalone
  four-edge probes remain unchanged, with no lost match or Go failure.
  Four crossing-detour differences
  are fixed: hierarchy ordering and P5 routing now share GraphInfoHolder's
  retained Random property even when cycle breaking consumed no random draws.
  Both reduced graphs, an enclosing container and a longer label match in all
  four directions (16 complete SVGs). A further 404,744 distinct four-edge
  cases expose two container-size differences: compound content hulls now
  initialize their maximum at negative infinity, as ComponentsCompactor does,
  instead of including zero when all P4 coordinates are negative. Both boards,
  an enclosing container and a longer label match in all four directions
  (16 complete SVGs). All 1,124,266 retained inputs replay with only these
  two fixes and no lost match; all 719,522 earlier outputs remain unchanged.
  The five-node enumeration is now complete: all 2,750,800 canonical
  three/four-edge cases match. E112 checks the remaining 1,830,460 cases
  without a difference, Go failure or incomplete output inventory. A fresh
  seed (2026110917) also matches all 64,000 labeled/parallel-edge cases,
  including longer labels. No further ELK finding remains in this lane.
  Exact coverage, inputs, outcomes and traces are under
  `oracle-cache/reports/e13/` through `e19/`, `e110/`, `e111/` and `e112/`.
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
- **TALA hunt container hierarchy (resolved):** `c08_cont40.d2` now
  matches with default seeds and seeds one through three. Hierarchy discovery
  ignores child incidences already reconnected to ancestors by the enclosing
  scope. A six-edge CLI contract and 96 direction, reversal and wrapper
  variants cover the fix (`oracle-cache/reports/T41/variants.json`).
- **TALA hunt cluster symmetry (resolved):** `m_mocha_soc_add.d2` now
  matches with default seeds and seeds one through three. Symmetry obstruction
  checks exclude the active cluster vessel's sibling slot by its vessel
  identity, preserving the symmetry credit used by Equidistance. The reduced
  Mocha input has a CLI geometry contract.
- **TALA hunt cluster transpose and lattice (resolved):** `c06_net80.d2`
  now matches with default seeds and seeds one through three. Its ten-edge
  reduction also matches: transpose resolves induced vessels through their
  owner cluster, runs `onlyFlip`, and
  restores policy and member geometry between speculative rotations. On the
  complete board, Swap and Transpose now recompute the owning lattice after
  cluster placement, as Go does; retaining 152 instead of 99 changed alignment
  scores. Both inputs have CLI contracts; 108 original, direction, reversal
  and wrapper replays improve from 45 to 108 matches without losing a match
  (`oracle-cache/reports/T42/replay.json`).
- **TALA dense-network performance (resolved on amd64):** the retained
  dense-network fixture and identical `c06_net80.d2`, seed 3, improve from
  18.87 to 8.99 seconds wall time (Go 11.97), with byte-identical SVGs.
  Shared node-identity indexes let projection reconciliation update the
  affected obstruction entries instead of scanning every retained snapshot.
  Sampled reconciliation CPU falls from 10.51 to 0.57 seconds. Peak RSS
  rises from 49.09 to 55.30 MiB (Go 55.03); speed takes priority.
  Nine rotating `c08_cont40.d2` repeats retain identical SVGs and remain
  below Go wall time. Arm64 confirmation remains; measurements and profiles
  are under `oracle-cache/reports/T410/`.
- **TALA small-graph enumeration:** all 7,328 cases match (two or
  three leaves, up to two disjoint containers and four directed edges,
  down/right).
  The two self-loop fan-outs now match: shifted subgraphs preserve loop
  clearance with explicit directions, as `Node.DoesOverlapAt` does. Candidate
  scores agree; the first difference was the compaction after sized iteration
  99. A CLI geometry contract and 64 direction, wrapper and seed variants
  cover the fix, with no lost matches. Canonical endpoint restorations now
  survive the later incidence rebuild, preserving the child-edge order used
  by Transpose's branch-size tie. This fixes 59 container cases; both retained
  container representatives match all 64 direction, wrapper and seed replays.
  A CLI contract covers the complete two-container geometry and routes.
  The final 72 container/self-loop differences now match too: hierarchy
  eligibility ignores self-loops when checking for internal connections,
  as pinned `hierarchy/eligibility.go` does. A CLI contract covers the
  retained triangle's complete 531 x 880 canvas, rectangles and routes.
  Complete enumeration outcomes are in
  `oracle-cache/reports/T47/enumeration/summary.json`.
  A 1,024-case four/five-leaf sample, including nested containers and
  edges incident to container vertices, now matches completely with no lost
  match. Internal connections include edges to the container
  itself, as Go's `Node.IsDescendantOf` does. A second CLI contract covers
  the reduced 540 x 572 layout; all 160 retained direction, wrapper and seed
  replays match. The two remaining witnesses now match too: sized swaps
  preserve self-loop clearance under explicit directions, as Go's
  `optimizerDoesOverlap` does. Initialization and all 77 sizeless iterations
  agree; the first difference was the swap after sized pass 85. Global
  symmetry subtraction now fuses only on arm64, retaining Go amd64's exact
  zero score and rejecting the chain's one-pixel Equidistance move. Both
  reductions have complete CLI geometry and route contracts; all 224 retained
  direction, wrapper and seed replays match with no lost match. Witnesses are
  in the lane reports directory; outcomes and traces are under
  `oracle-cache/reports/T46/`.
  The clustered-container transpose witness now matches Go's complete
  586 x 733 SVG instead of widening it to 850 px. Transpose synchronizes
  the vessel's dimensions after wrapping its container members, as
  `Transaction.Commit` and `Cluster.SyncGeometry` do. The previous cached
  width understated rotation costs and admitted a move Go rejects. A CLI
  contract covers every rectangle and route. The additional 1,022-case
  sample now matches completely, up from 1,021, and all 256 retained
  direction, wrapper and seed replays match, up from 242, with no lost match
  (`oracle-cache/reports/T47/{contract-comparison,replay}.json` and
  `fresh/summary.json`). Fresh seed 2026110947 also matches all 1,022
  four/five-leaf cases, with no findings or lost match
  (`T47/new-sample/summary.json`).
  Seed 2026110948 now matches all 4,088 cases, up from 4,080 with
  no lost match. Compound proxies enter hierarchy placement without
  Rust's eager tree-routing state, as Go's fresh `NewGraph` does. This keeps
  the incoming-edge traversal and placement shuffle in Go's order; a CLI
  contract covers every rectangle and route. Nested opposing routes now
  retain the OVG search's selected ports, as Go's `generateRoutes` does;
  removing Rust's extra direct-tunnel search fixes the exchanged source ports.
  A CLI contract covers the complete canvas, rectangles and routes. The final
  container fan-in finding now matches too: cluster flips read the retained
  child endpoints and their actual graph ownership, as Go's
  `clusterExternalConnectedNodes` and `SetGraphReference` do. Initialization
  and all 126 annealing iterations already agreed; the first difference was
  the following cluster flip. A CLI contract covers the complete 852 x 862
  canvas, every rectangle and route. The sample replay loses no match.
  Sources and comparisons are under `oracle-cache/reports/T48/`, `T49/`
  and `T410/`. Fresh seed 20261109410 also matches all 2,048 four/five-leaf
  cases, with no findings or lost match (`T410/final/fresh/summary.json`).
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
- **Dagre container ingress and reversed self-loops (resolved):** separate
  external sources no longer collapse sibling children into one lane. The
  obsolete flat-port box and route corrections are removed. Compound graphs
  also retain their border-based translation when self-loops are restored,
  preserving routes and container padding under up/left flow. Nested feedback
  edges also keep Dagro's cross coordinates without an added quarter pixel.
  Both hunt repros, connected-child and outgoing variants match in all four
  directions; 136 nested feedback variants match too
  (`oracle-cache/reports/B11/{final,quarter-variants}.json`).
- **Dagre small-graph routes (retained findings resolved):** final clipping
  now preserves the fractional container bounds written by `fitPadding`.
  Border-model layouts also skip the obsolete outside-node lane shift,
  preserving reciprocal container routes. Both retained witnesses match in
  all four directions. Enumeration matches all 8,860 two/three-node jobs
  through four edges and all 464,640 four/five-node jobs through three edges
  (60 retained differences fixed), including all 380,250 five-node three-edge
  jobs. All 155,040 four-node four-edge jobs now match too. B17 adds the
  first 100,000 five-node four-edge matches (73,840 SVGs and 26,160 errors),
  and B18 adds 1,000 matching errors, bringing cumulative coverage to
  729,540 outcomes with no differences,
  timeouts or signals. No previous match is lost.
  Five-node enumeration through four edges remains incomplete;
  continue at index 720,680. B18's additional rebase replay matches 244 SVGs
  and 12 errors from the beginning of the queue. Repros and outcomes:
  `oracle-cache/reports/B12/`, `oracle-cache/reports/B13/` and
  `oracle-cache/reports/B14/` through `oracle-cache/reports/B18/`.
- **Connection gradients and label paints (resolved):** the private raster
  bridge retains gradient references, original per-path paint identities,
  local arrowhead bounds and transforms, and each label line's measured ink
  box. A 240-case PNG matrix improves from 72 to 240 exact Go outcomes,
  including linear/radial paints, sketch, two themes and all three engines.
  Three CLI contracts cover the original edge/text loss and multiline labels
  (`oracle-cache/reports/B18/`). A separate sketch shape-stroke probe still
  has 16 one-channel pixel differences, unchanged from the baseline; its
  constant-color gradient also differs, pointing next to fill/stroke coverage
  and compositing rather than gradient coordinates.
- **Crate packaging (resolved):** CLI declarations, help and graph field
  schemas are Rust source instead of embedded oracle JSON/text files.
  The library package excludes 14 oracle/diagnostic example drivers.
  Both package inventories retain their runtime sources and omit the former
  internal material (`oracle-cache/reports/B18/package-{lib,cli}.log`).
- **Fresh structure fuzz findings (F11–F14):** Dagre/ELK/TALA seeds
  2026110871/73/75 produced six output differences in 3,000 SVG jobs, plus
  one malformed input that panics in both CLIs. The ELK SQL-table self-loop
  finding is fixed (F12): retain RoutingDirector's selected boundary ports
  before endpoint adjacency is restored, as elk-go's HyperLoop does. The
  original ELK seed now has 993 rendered matches, six empty-output matches
  and only the shared panic. Fresh Dagre/ELK seeds 2026110877/79 match all
  2,000 jobs; TALA seed 2026110881 initially had four placement differences.
  F15 fixes its horizontal nested-ID reduction: hierarchy container sync now
  performs `WrapChildren`'s boundary-label expansion before fitting, preserving
  the extra pixel after transposition. The exact right/left reductions match,
  down/up remain unchanged matches, and the full seed improves to three
  differences with no other outcome change. Fresh seeds 2026110883/85/87 found five
  output differences in 3,000 jobs plus one malformed input that panics in
  pinned Go first. The visible ELK nested-sequence difference is fixed (F14):
  sequence roots use the final synthetic lifeline endpoint for height, as
  `d2sequence.getHeight` does, instead of extending to nested group bounds.
  F17 fixes all three F16 route-only rows: ordinary TALA
  endpoints now pass their actual port inventories to `centerSymmetricalPorts`,
  matching pinned Go instead of treating the inventories as empty whenever no
  cluster ports were saved. Retained TALA seed 2026110881 improves from 991
  rendered matches, six empty-output matches and three differences to 994/6/0;
  fresh seed 2026110893 improves from 987/12/1 to 988/12/0. Fresh Dagre/ELK
  seeds 2026110889/91 add no parity differences in another 2,000 jobs (one ELK
  malformed input produces the same invalid output on both sides). F19 fixes the
  last retained TALA row, F11 job 508. Symmetry replacement and obstruction now
  use the active cluster vessel's position and size together, matching pinned Go
  `placementcost.nodeSymmetry`; seed three and the default result now match. The
  complete historical seed `2026110875` now has 993 rendered matches, seven
  empty-output matches and zero diffs, so all retained TALA structure-fuzz rows
  are resolved. F110's Dagre PNG seed 2026110895 found two severity-3 rows in
  1,000 jobs. Entity-bearing single-key links now keep Go's empty PrettyLink
  and no longer invent a numbered raster appendix. F111 fixes the remaining
  row: bounded PNG legend scenes are completed before their pixels are streamed
  to the encoder, so the panel no longer loses its title and every entry. The
  seed now has 990 matches, ten empty-output matches and zero differences. The
  F112 ELK PNG seed 2026110897 found one severity-2 compound self-loop route.
  Opposing loop ports now follow a nested graph's transferred fixed boundary
  port at ELK's 10 px port spacing, matching PortRestorer and
  LabelAndNodeSizeProcessor; the seed improves from 991 rendered matches,
  eight empty-output matches and one difference to 992/8/0. F113 expands the
  format sweep: Dagre/ELK PPTX seeds 2026110901/03 match all 2,000 jobs. TALA
  PPTX seed 2026110905 initially had three differences; its visible class/grid
  row is fixed by using the exported structured target font in the private
  raster shape, matching `d2exporter.toShape`, and the seed improves from
  997/3 to 998/2. The two residual PPTX rows are severity-4 single-pixel
  gradient and emoji/Markdown differences. TALA PNG seed 2026110899 has one
  Unicode text-raster difference handed to D4; its minimized witness is
  `.reference/codex-runs/witness-D4-F113-tala-png-rtl-japanese.d2`. The
  F114 PDF sweep fixes TALA's severity-2 empty nested-grid row: childless grids
  retain their D2 label metrics but enter the core TALA graph as ordinary
  leaves, matching `LayoutNested` and `d2talalayout.translateGraph`. F115 fixes
  the remaining glob/flow row: horizontal hierarchy rank spacing uses an edge
  label's width, as pinned TALA's
  `minimumDistanceToNextLevel(..., isHorizontal)` does, instead of its height.
  Seed 2026110923 now matches all 1,000 jobs. ELK's only PDF finding is
  a malformed input that panics in pinned Go, and Dagre's structural PDF row
  is the already-recorded subpixel route difference. The 18 eligible GIF and
  18 eligible TXT smoke rows match across all engines. F116 fixes the retained
  Dagre coordinate-frame row: Dagro's graph translation
  includes restored self-edge label boxes (and their parallel-edge gap) before
  D2 rounds object origins, while routes keep the exact fractional translation.
  Seed 2026110871 now has 987 rendered matches, 13 empty-output matches and no
  differences. F117 fixes the final ELK half-pixel route row: cloud inner-box
  padding now ceils the leading placement before deriving trailing insets, as
  pinned D2's `shapeCloud.GetInnerBoxForContent` does. Seed 2026110885 now has
  985 rendered matches, 14 empty-output matches and only the shared malformed
  input panic. Inputs, traces and exact outcomes are under
  `oracle-cache/reports/f11/` through `f117/` and the lane reports.

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
- `scripts/shape-trace-differential.py`: seeded small graphs of every
  non-rectangular shape at sizes around the wedge thresholds, with parallel,
  reversed and labeled edges and containers, on dagre and ELK. Its first run
  (2026-11-11, 3 seeds x 400 cases) found ovals with one authored dimension
  sized without `shapeOval.GetDimensionsToFit`'s aspect limit (fixed; every
  ELK case now matches) and a dagre ordering difference when opposing edges
  between one pair carry a label after an unrelated node (fixed: the flat
  ordering path now pairs reversed dummies per entry pair like dagro's
  sortSubgraph). Eight seeds (6,400 renders) then matched Go. Dagre and ELK
  now clip edges with the Go-exact `shape.TraceToShapeBorder` port; the older
  approximation is gone.
- `scripts/graph-differential.py`: the same harness over a `wide` family of
  ordinary diagrams (nested containers and grids, per-container directions,
  label and icon positions, near constants, arrowheads and edge label
  positions, style keywords, Markdown and code labels, SQL tables, classes,
  sequence diagrams, `--sketch` and `--theme`) on dagre, ELK and TALA, with
  line-level minimization of each mismatch. Its first run (2026-11-11, 4 seeds
  x 300 cases) found 8 dagre, 2 ELK and 8 TALA mismatches; the dagre ones
  reduce to `label.near` center-left/right on container children and
  outside label positions on containers.
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
  A fresh 1,000-job Dagre PDF run (seed 20261108251 with the same broad
  sketch/theme/scale/center flags) initially found six differences. Empty
  Markdown h1/h2 blocks now follow Go's zero-inline-height path while still
  painting their padding and rule; the reduced PDF is byte-identical after
  the fix. Leading raw `<br/>` elements in headings now preserve their empty
  first line, and self-closing non-void tags follow `x/net/html`'s implicit
  fragment-EOF close. Their SVG and pixels match Go. Raw labels containing
  XML-forbidden controls now retain their d2scene shaping text across the
  private SVG bridge; the form-feed witness matches as SVG, PNG and PDF.
  Exact replay is 999/1,000; the escaped edge-key row also matches without a
  related code change, and the only remaining difference is one of Go FPDF's
  unsorted two-font map orders. Reductions and replay summaries are under
  `oracle-cache/reports/d432/`, `d433/` and `d434/`.
  A fresh 1,000-job Dagre PPTX run (seed 20261108261 with broad
  sketch/theme/scale/center flags) initially found eight embedded-PNG
  differences. Private structured class/SQL text now carries its exact raw
  source into native shaping, preserving leading spaces that usvg otherwise
  collapses. Markdown flow now discards the collapsible source newline
  between consecutive raw `<br/>` elements before grouping inline units, so
  every empty line box survives raster export. Exact replay improved from
  992/1,000 to 994/1,000; the six remaining legend, gradient, paper, edge-label
  and border-label image differences are structural/rendering witnesses for
  the structure lane. The summaries and reductions are under
  `oracle-cache/reports/d435/`.
  A fresh 1,000-job Dagre TXT run (seed 20261108271) has 992 rendered and
  eight empty-output matches with no functional findings. Two apparent
  error-text differences were identical missing-child-output diagnostics
  containing independently generated `.d2-board-output-<nonce>` paths; the
  fuzzer now masks that ephemeral nonce. Initial and exact-replay summaries
  are under `oracle-cache/reports/d436/`.
  A fresh 1,000-job ELK GIF run (seed 20261108273, sketch, themes, padding,
  scale and centering) has 999 matches and one structure finding: duplicate
  wildcard width assignments leave Go's node at its 100 px default while
  Rust applies width zero. A matching ELK PNG run (seed 20261108275) has 984
  rendered matches, 14 empty-output matches and two instances of the existing
  legend structure cluster. All three reductions are retained under
  `oracle-cache/reports/d437/`; the wildcard witness is also handed to the
  structure lane as `witness-F1-D437-duplicate-glob-width.d2`.
  Fresh ELK PDF and PPTX runs (seeds 20261108277/279, 1,000 jobs each, the
  same flags) have 995 and 997 matches. Their eight differences contain no
  missing or changed text: direct SVG/PNG output is exact, while the paged
  raster differs only in structured SQL/sequence, gradient and animated-grid
  paint. The reduced long-label sequence differs in four PPTX image pixels at
  a connection stroke. Results and reductions are under
  `oracle-cache/reports/d438/`; the SQL-opacity and gradient reductions are
  retained as `witness-F1-D438-{sql-opacity-pdf,gradient-pptx}.d2`.
  Fresh TALA PDF and TXT runs (seeds 20261108281/283, 1,000 jobs each) have
  998 and 999 complete-output matches respectively, plus seven matching empty
  TXT outputs. Neither found a text-content difference. One PDF row repeats
  the known paged sequence-paint cluster with byte-identical direct SVG/PNG;
  the other differs already in TALA layout. The TXT row reduces to three
  adjacency lines and differs at one connector cell, with different TALA SVG
  geometry too. Results are under `oracle-cache/reports/d439/`; the new
  structural reductions are retained as
  `witness-F1-D439-tala-{pdf-layout,txt-edge}.d2`.
  A fresh TALA GIF run (seed 20261108285, 1,000 jobs with sketch, themes,
  padding, fractional scale, centering and animation) matches completely.
  A matching PNG run (seed 20261108287) has 981 rendered matches, 16 matching
  empty outputs and three output differences. One is the existing legend
  raster-paint family (SVG exact; 405 pixels differ); two already differ in
  TALA SVG geometry, including one canvas-size change. No text or content is
  missing. Minimized reductions and summaries are under
  `oracle-cache/reports/d440/`; structure-lane witnesses are retained as
  `witness-F1-D440-tala-{legend-png,nested-edges,sql-edges}.d2`.
  A fresh TALA PPTX run (seed 20261108289) has 995 matches and five
  structure/paint differences with identical text inventories: two repeat the
  gradient-stroke cluster, one repeats paged sequence paint, and two reduce to
  TALA route and malformed SQL-table layout. The new structure witnesses are
  `witness-F1-D441-{tala-route,sql}-pptx.d2`. ELK TXT seed 20261108291 has
  990 rendered matches, nine matching empty outputs and one input where both
  implementations panic while compiling malformed theme overrides. The fuzzer
  now records such inputs in `skips.jsonl` with Rust's outcome instead of
  reporting a Rust-only finding; its exact replay has zero findings. Reports
  and reductions are under `oracle-cache/reports/d441/`.
  A fresh Dagre GIF run (seed 20261108293, 1,000 jobs with sketch, themes,
  padding, fractional scale, centering and animation) matches completely.
  TALA SVG seed 20261108295 has 990 rendered matches, six matching empty
  outputs and four structure-only differences. All four reductions retain
  identical text inventories and differ first in TALA paths; their minimized
  child/parent, descendant-label and adjacency witnesses are
  `witness-F1-D442-tala-{child-parent,descendant-label,adjacency-a,adjacency-b}.d2`.
  Summaries and reductions are under `oracle-cache/reports/d442/`.
  A fresh Dagre PNG run (seed 20261108297, 1,000 jobs with sketch, themes,
  padding, fractional scale and centering) has 987 rendered matches, 11
  matching empty outputs and two PNG-only paint differences. The paper-pattern
  and legend rows have byte-identical public SVGs, including text and geometry,
  and repeat the existing structure/rendering families rather than exposing a
  text-content regression. Results and reductions are under
  `oracle-cache/reports/d443/`.
  A fresh ELK SVG run (seed 20261108299, 1,000 jobs with sketch, themes,
  padding, fractional scale and centering) has 991 rendered matches and nine
  matching empty-output inventories. It found no output, text, error, panic,
  signal or hang difference. Results are under `oracle-cache/reports/d444/`.
  A fresh Dagre PPTX run (seed 20261108301, 1,000 jobs with sketch, themes,
  padding, fractional scale and centering) has 994 matches and six output-only
  differences. All six have byte-identical public SVGs and identical text
  inventories. Five also have byte-identical direct PNGs and repeat the paper,
  sequence, nested-3D and border-label paged-paint families. The sixth differs
  only on rasterized grid-cell borders; its minimized structure witness is
  `witness-F1-D445-grid-lines-pptx.d2`. Results and reductions are under
  `oracle-cache/reports/d445/`.
  A fresh ELK PDF run (seed 20261108303, 1,000 jobs with sketch, themes,
  padding, fractional scale, centering and omitted version metadata) matches
  completely. It found no output, text, error, panic, signal or hang
  difference. Results are under `oracle-cache/reports/d446/`.
  A fresh Dagre SVG run (seed 20261108305, 1,000 jobs with the same broad
  visual flags) has 989 rendered and 11 empty-output matches with no finding.
  ELK PNG seed 20261108307 has 989 rendered matches, six empty-output matches
  and five output-only differences. All five minimized reductions retain
  identical text inventories. Four have byte-identical public SVGs and reduce
  to C4-person, legend and gradient painting; the sequence reduction differs
  in canvas height and frame geometry while retaining all six text nodes.
  Structure-lane witnesses are `witness-F1-D447-{c4-person,legend,sequence,
  gradient-fill,gradient-stroke}-png.d2`; results are under
  `oracle-cache/reports/d447/`.
  A separate deterministic text-mutation family now appends Markdown, code,
  LaTeX, mixed Unicode, link and tooltip labels without changing the existing
  structural seed stream. Dagre PPTX seed 20261108309 and ELK PDF seed
  20261108311 (1,000 jobs each with broad visual flags) initially had 12 and
  eight output differences. Seven rows reduced to `e` + ZWJ + Arabic alef:
  public SVG and direct PNG were exact, but paged output used the wrong font
  because its bundled resolver did not continue to the host fallback. Paged
  default-ignorable labels now use the typed scene shaper and the same bundled
  then system fallback chain as pinned `d2cli.newFontFallbackOptions`. Exact
  replay improves to 992/1,000 PPTX and 995/1,000 PDF matches with no lost
  match; all 13 remaining differences have exact text inventories and repeat
  known structure/paint families. The minimized text witness is
  `witness-D4-D448-pdf-zwj-arabic.d2`; reports are under
  `oracle-cache/reports/d448/`.
  Fresh text-family Dagre PDF and ELK PPTX runs (seeds 20261108313/315,
  1,000 jobs each with the same broad visual flags) have 993 and 989 exact
  matches. The seven PDF differences have byte-identical public SVGs and
  identical extracted PDF text. All 11 PPTX differences have byte-identical
  public SVG text and geometry; ten reproduce as direct-PNG paint differences
  and one is paged-only. No text, label, heading, code line, tooltip, link or
  board content is missing or changed. Results and cross-format probes are
  under `oracle-cache/reports/d449/`.

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
  eagerly dropping the arena walks every allocation. Releasing only graph,
  class, validation and IR lookup caches is CPU-neutral but does not lower the
  roughly 480 MiB peak: the lexical arena itself is the overlapping storage.
  Both candidates were rejected. Closing this needs bulk-owned compiler arena
  storage, or consume-on-lower ownership, so release does not walk every IR
  allocation. Shadows,
  opacity, 3D, nested fills and dark theme stay under Go but
  their ratios grow with size. The corpus itself has no row over Go.
- **Long compile progress:** the CLI now mirrors pinned `d2cli.compile`'s
  five-second `info: compiling & running layout algorithms...` ticker and
  cancels it on every compile/layout return path. The dense/800 TALA
  edge-limit row now has byte-identical stderr and exit status instead of a
  parity-error mismatch.
- **Unreplayed witnesses:** the original four LaTeX probes and three fuzz
  singletons (`*: suspend` globs, a `package` shape four levels deep,
  Gujarati next to an emoji) were not retained; 60 `suspend` and 18
  `package` variants match. Replay them if a sweep finds them again.

Go progress-message count differences that are not reproducible when each CLI
runs alone remain harness timing noise; missing the ticker entirely was real
drift and is fixed above.
Historical run details are in the lane reports and d2-rs-history.

## Verification

Run `cargo test --locked --workspace` for the self-contained default tier and
`scripts/test-oracle.sh` for the oracle tier. The e2e, differential and
performance scoreboards and their prerequisites are documented in
[SCOREBOARD-WORKFLOW.md](SCOREBOARD-WORKFLOW.md). The last qualified e2e
baseline was 944 of 944 applicable boards matching (933 SVGs and 11 expected
errors). [BENCHMARKS.md](BENCHMARKS.md) describes the performance baseline.
