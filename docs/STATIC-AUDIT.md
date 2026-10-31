# Static parity audit — D2 v0.9.0

Evidence JSON cited below as `docs/*.json` is no longer in the tree. It was
retired at 9b706ac1 and in the 2026-10-31 second pass; `docs/RETIRED-EVIDENCE.md`
lists each file and how to read it back. Checkers now write fresh reports
under the gitignored `oracle-cache/reports/`.

This is a source inventory, not a behavioral parity claim. It records the
comparison used to choose the current reverse-engineering slice.

The pinned upstream checkout is `/Users/nelson/code/d2` at
`d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda`, built with Go 1.27.0. The
instrumented copy is `/Users/nelson/code/d2-v0.9.0-trace`; it is kept separate
from the pristine checkout. `scripts/audit-tala-structure.py` currently sees
201 non-test Go TALA files and 197 Go test files mapped into 102 Rust internal
files. Its symbol list is intentionally conservative: Rust modules collapse
Go files and rename methods, so missing names are review leads rather than
missing implementation proof.

The first target comparison exposed a concrete gap in `d2-rs-export`:
`render_ascii` projected node centers into coarse `[label]` cells, while
`d2renderers/d2ascii` calibrates every shape and route against a 9.75px ×
18px font cell. The Rust implementation now mirrors the source drawing order,
label-aware calibration, route-only row compaction, protected text cells,
directional arrows, and the upstream primitives for rectangle, page, hexagon,
package, parallelogram, cylinder, document, queue, step, callout, stored-data,
person, diamond, oval, circle, and cloud boundaries. The two basic witnesses
below are byte comparisons against the Go binary, not visual approximations:

```text
a -> b
a -> b: hi
```

The editor comparison then traced the pinned `d2oracle/edit.go` surface. Its
public `Create`, `Set`, `Delete`, `Rename`, `Move`, and `ReconnectEdge` methods
mutate compiler-owned AST references and recompile the graph; source-text
replacement alone cannot preserve those scopes, imports, descendants, and
edge identities. The current Rust slice therefore starts with AST-backed root operations
(`create_root_field`, `set_root_field`, `delete_root_field`, and
`rename_root_field`) plus a source-level single-edge `reconnect_edge` operation
that walks nested value maps. These operations parse, mutate, format, and parse
again while preserving written labels, arrows, and edge indices. Nested boards,
compiler-aware references, descendant propagation, glob/import updates, and
full graph edge semantics remain explicit audit targets rather than being
hidden behind a broad but incorrect editing API.

The same source-first pass found that native Markdown labels were leaving HTML
character references literal. D2's Go renderer decodes the complete Go 1.27
`html/entity.go` table before text measurement and font-subset collection. The
existing generated table is now owned by `d2-rs-text` and shared by import URL
validation; named, numeric, legacy, and escaped references have an exact text
run witness against the Go 1.27.0 oracle, recorded in
`docs/markdown-entity-parity.json`.

The renderer error boundary also differed for raw HTML: the Go source build
rejects `<u>`, `<span>`, and similar unsupported inline elements instead of
painting them as literal text. The checked CLI/export/WASM source path now
validates the generated HTML tag set and returns the same deterministic error;
the paired error report is `docs/markdown-raw-html-error-parity.json`, while
full raw-HTML scene support remains a separate translation slice.

The v0.9.0 SVG source audit also found that marker IDs are not arbitrary local
hashes. `d2renderers/d2svg.arrowheadMarkerID` hashes the tuple
`arrowhead,isTarget,strokeWidth,stroke` with FNV-1a 32 after appending the
literal `lalalas` secret; the default target-triangle tuple is `3488378134`.
The Rust dynamic-marker path now uses that exact tuple spelling and hash width,
with a regression against the upstream value. The remaining target boundary
work is larger: D2 first hashes the complete `d2target.Diagram.Bytes()` value
into `d2-<hash>`, then prefixes marker, mask, shape, and appendix IDs with that
diagram hash. Rust now reconstructs that compact target byte stream for the
common graph fields, including Go's integral-float and HTML-escape spelling;
the simple `a -> b` target hash matches Go both with and without salt. Rust
still uses its semantic target projection and local ID rewrite for the final
SVG, so structured target fields and exact per-object ID naming remain open.
The config witness also follows the source boundary: changing `theme=1` changes
the salted simple-edge hash (`3486127812`), while `pad` and `center` do not,
because those are renderer canvas options rather than target identity fields.
Common and salted arrowheads now use the upstream
`mk-d2-<diagram-hash>-<marker-hash>` form, and every edge uses the upstream
`d2-<diagram-hash>` mask ID; node/connection identity groups use the same
escaped URL-base64 classes. Multi-edge mask geometry and structured clip paths
remain open.
Dynamic marker definitions now appear at the first connection that uses them,
as in `d2renderers/d2svg.drawConnection`; the remaining source-space viewport
origin now follows the source's nested SVG boundary; multi-edge mask-bound
calculations are still under comparison.
The target projection also now follows `d2exporter.toShape`: graph-circle
remains an `oval` target shape, and layout-computed label positions are omitted
when the source label is empty. The explicit-style `geometry.d2` hash matches
the Go 1.27.0 witness.
Icon fields now use the target's exported `url.URL` shape in the hash stream;
image defaults (`fill=N7`, centered icon position) are included as well.
Text/code target defaults and the Go exporter’s float32 route truncation are
also applied before hashing, including the zero default for an unspecified
edge label percentage.

The same source comparison found that the admitted raw inline tags (`<b>`,
`<em>`, `<s>`, `<strike>`, and `<br />`) were accepted by the validator but
still entered the Rust label ruler as visible glyphs. The native inline parser
now tracks their style stack and line breaks, and plain-text projection omits
tag spelling. The focused witness compares the Go and Rust text-run geometry
and is recorded in `docs/markdown-raw-inline-parity.json`.

The pinned `d2lib.applyConfigs`/`applyDefaults` boundary also applies root
`vars.d2-config` values before the public JavaScript compile/render path is
executed. The WASM source boundary now preserves that precedence for layout
engine, TALA seeds, theme and dark theme, padding, centering, sketch mode, and
palette overrides; explicit JSON options remain authoritative. The source
default is Dagre, while explicit `engine: "tala"` remains available for
TALA-specific callers. Focused WASM regressions cover omitted-engine parity,
root layout-engine selection, configured seed selection, and render config
hydration.

The same comparison covered Markdown images without an asset provider. The
native path omits the image URL and alt text, collapses the visible separator,
and preserves the Go ruler's legacy width accounting. The pinned witness and
exact viewport/text-run comparison are recorded in
`docs/markdown-image-parity.json`.

The same audit found that the ELK layered adapter was passing an explicit
70px rank span to the Dagre baseline. That discarded D2's connection-label
lane. Layered ELK now leaves the rank span unset so the shared source-shaped
calculation uses its 100px base plus the largest label lane. This change is
observable in labelled ASCII and SVG geometry and is covered by the existing
workspace tests.

The ELK source audit then compared `d2layouts/d2elklayout/layout.go` and its
deprecated WASM `ConvertGraph` path. In addition to table-side and dynamic-port
rules, the converter reserves 40px per incident port on the flow cross-axis and
adds a label's height plus 5px when an icon shares the node. Rust now computes
those effective sizes locally while serializing `getELKGraph`, preserving the
measured graph and covering both vertical and horizontal flow in a focused
regression. The same pass now follows `positionLabelsIcons` defaults for
container label/icon padding and carries effective dimensions into the ELK
minimum-size option. Full elk-go placement and section routing remain open.

The native ELK preparation initially collapsed all self-loop label sizes into
one global maximum. The pinned source stores maxima by owning container; Rust
now retains that map and uses only root-owned loops for root clearance, with a
focused nested-loop regression. Applying each nested owner's local clearance
still belongs to the compound elk-go translation.

The separate elk-go self-loop processor chain is now represented at the
source-shaped routing boundary for direct node endpoints. Rust assigns the
implicit loop endpoints to the north side, applies ELK's distributed port
positions, reserves `nodeSelfLoop` clearance, and emits the two-bend
orthogonal loop through all four root directions. Multiple direct loops now
follow the source's outward slot ordering and edge-to-edge lane spacing. Loop
labels now contribute the flow-axis extent and edge-label spacing and are
published in the matching lane. Fixed north-port loops reuse the same slot and
route boundary. Nested scopes retain their local default clearance and resize
the parent compound. The focused witnesses match the Go 1.27.0 output; other
fixed port constraints and deeper compound-local loop margins remain open.

The distribution follow-up found an earlier routing-baseline omission:
`loops/routing/orthogonal_router.go::baselinePosition` adds the existing
side-specific node margin before `nodeSelfLoop`. Rust now includes all four
imported margins in its outer bends. An asymmetric-margin regression with
a nonzero node translation checks the four normal-axis coordinates. This
does not qualify margin production, placement, or slot-dependency ordering;
the current node alignment still reserves clearance as a north-only band.

The retained loop matrix is now exact across the expanded self-loop corpus.
The source-shaped placement path carries original and computed loop margins
through `RouteOrthogonal` before `placeNodesHorizontally`, while the restored
routes retain the source side ordering. Fresh runs of
`scripts/check-elk-loops.py` pass 96/96 for flat and nested graphs, with and
without labels, and for the multiple-label plus nested label-spacing witness.
This qualifies the loop wire geometry and root envelopes in those matrices;
general hierarchy, detailed traces, graph RNG integration, and the remaining
ELK processor family still require separate qualification.

The next ELK comparison caught an outside-label sizing error in the native
adapter. `d2elklayout.Layout` adds measured height only for a label that shares
an icon; it serializes ordinary outside labels as `ELKLabel` records without
enlarging the node rectangle. Rust had been adding every outside label lane to
the measured box, which made a default person node 92px tall instead of the
source's 66px. The preparation pass now follows the Go rule, and the 16-shape
ASCII corpus is byte-identical again. The shared graph also retains source
node-label text in adapter-only metadata so native ELK serialization carries
the same label records as the Go converter without changing public GraphJSON;
structured text, class, SQL-table, and code shapes remain excluded by the
same `HasLabel` rule as the source.

The ELK graph records were then moved from the WASM compatibility module into
`d2-rs-elk::model`, including the section and bend-point fields used by
elk-go's transfer boundary. Both native and WASM code now share the same
serialized schema; this is a structural prerequisite for translating the
layered phases, not a claim that the current Dagre-backed layout is elk-go
equivalent. The records implement both serde directions so the forthcoming
native codec can import the same graph that the WASM boundary exports.

The native ELK codec now mirrors the pinned `ConvertGraph` transfer boundary
in `d2-rs-elk::codec::graph_to_elk_graph`: root/container option keys,
hierarchy order, measured node and edge labels, SQL-table fixed ports, and
dynamic table-level port IDs are emitted from the shared graph. The companion
`graph_to_elk_graph_with_table_columns` accepts the compiler's source column
names for exact `srcPortID`/`dstPortID` identity. Its tests are boundary tests
only; they do not assert elk-go placement or section routing. The paired
`apply_elk_graph_result` path copies the Go adapter's relative-coordinate,
ceiling, and edge-section container-offset rules into `LayoutResult`, with a
nested-node regression. This leaves the actual layered solver as the explicit
remaining parity gap.

The first solver slice now follows the pinned `elk-go` internal arena instead
of treating the wire model as the algorithm's data structure. `layered.rs`
imports stable node/edge identities and nested graph scopes, ports the
`GREEDY_MODEL_ORDER` cycle breaker (including sink/source queues, maximum
outflow tie selection, and edge reversal), and ports the ELK 0.8.1-compatible
longest-path layerer with prepended rank allocation. Tests assert the same
chain, branch, self-loop, and two-node back-edge states. The arena is exposed
for subsequent phase comparisons; production `--layout=elk` still uses the
existing Dagre baseline until ordering, placement, and routing are migrated.

The same arena includes a bounded phase-three probe: a stable barycenter sweep
and adjacent-layer crossing counter. It compares the core ordering rule from
`p3order/barycenter_heuristic.go` on a minimized crossed-edge witness while
preserving model order for nodes without fixed neighbors. ELK's port-rank
distribution, Forster constraints, randomized sweep schedule, and
hierarchy-aware processor remain pending; the probe is kept separate from the
production Dagre-backed path.

The arena also has the first bounded phase-four primitive from
`p4nodes/simple.go`: `place_layers_simple` computes each layer's measured
cross-axis extent, centers shorter layers around the tallest, and assigns
source-shaped stacked coordinates with explicit spacing. ELK node margins are
now imported and included in the size arithmetic. It has a direct two-node
regression; Brandes–Köpf horizontal placement and hierarchy-aware compound
handling are still open.

The next phase-four slice is `build_bk_alignment`, which ports the core
median-neighbor block construction from `p4nodes/bk_aligner.go` over the
port-aware arena. It records circular block roots and alignment links in
model order and honors marked-edge exclusions on a minimized diamond
witness. `mark_bk_conflicts` now ports the type-one interval rule with a
three-layer long-edge witness. ELK's full port-anchor shifts, horizontal
compaction, balanced layout, and production integration remain pending.

`apply_bk_inner_shifts` now ports the next source operation over the retained
port geometry: it walks each circular alignment block, computes source/target
port-center deltas, includes top/bottom margins, and records the block's
cross-axis extent. Full mutable anchor semantics and threshold straightening
are still pending.

`compact_bk_alignment` adds the first difference-constraint placement pass:
same-layer order and spacing are applied to aligned block roots while
preserving the source traversal direction. `choose_bk_layout` now evaluates
the four source directional candidates and applies the balanced median with an
order check. The directional path also solves the source sink-class DAG and
implements fixed BK alignment choices; balanced and left/up v0.9.0 diamond
witnesses match the Go coordinates. Threshold straightening, compound
margins, and wiring this phase into the public ELK adapter are still open.

The size boundary now also follows `intermediate/layer_size.go` for the
retained geometry: widest layer, first-to-last cross-axis extent, graph
height, and vertical offset are recorded after placement. External-port
surrounding margins and complete graph-width propagation remain pending.

`split_long_edges`, `route_polyline_edges`, and `join_long_edges` now follow
the v0.9.0 long-edge/routing boundary: the split allocates zero-size dummies
in each intervening layer and gives them west/east ports in stable order; the
router writes ordered in-layer and port-exit bends; the join restores the
incoming edge's final target, retires synthetic segment slots, and carries
bend/junction metadata across the join. Label transfer, thickness, full
junction semantics, direction transforms, and production ELK wiring remain
open. The public `layout`/`route_edges` functions still use the deterministic
Dagre adapter; `layout_elk_graph_with_layered_pipeline` is the separate
source-shaped arena integration seam used for qualification.

The next source comparison found that ordinary inter-layer edges without
explicit ports were still emitted as diagonals by the translated router. The
Go `elk-go` profile inserts one shared lane in the layer gap when endpoint
cross-axis coordinates differ. Rust now adds the corresponding two ordered
bends after final node placement, while preserving the existing explicit-port
and in-layer behavior. A minimized diamond regression asserts the lane and
endpoint ordering; this is an arena-phase fix and does not claim production
ELK parity.

The comparison also separated the two ELK spacing properties that had been
collapsed in the qualification entry point. D2's configured rank gap controls
between-layer placement, while the default 20px node-to-node spacing controls
the cross-axis stack. The source-shaped path now passes those domains
separately; the full orthogonal provider still needs its implicit ports,
post-placement repeat, and compound processors before it can replace the
public Dagre-backed path.

The qualification seam now performs the importer-side default-port step as
well: direct node endpoints receive zero-sized east/west ports at the node
center before layering. This is isolated from transfer-boundary tests, which
still preserve unresolved endpoint IDs; explicit side-constrained ports are
positioned from their side and index in the same preprocessor.

Those synthesized ports are then distributed with ELK's default
`DISTRIBUTED` alignment, and the qualification path applies the orthogonal
router's two-sided edge-node clearance. The unequal-height diamond witness now
matches the pinned Go node boxes and route points up to the root translation;
the two-by-two crossing witness also matches the three lane slots and split
bends. More complex dependency cycles and later postprocessors remain separate
work.

The same seam now transposes node dimensions, margins, and port frames for
DOWN/UP roots before running the horizontal phases, then applies the inverse
direction transform at publication. A DOWN diamond matches the pinned Go
geometry and routes up to the root translation.

Compound and explicit-port witnesses now close the next transfer/phase gaps:
the one-level nested graph matches child packing, parent sizing, center
alignment, and the straight external route; explicit east/west ports match
`elk.port.index`, distributed anchors, west-side ordering, and the ordinary
rank gap for direct port edges. These are exact against elk-go after removing
the oracle's 12px root padding; a recursive two-level compound witness now
matches too, and a DOWN north/south-port witness matches after publishing the
inverse port-frame transform. External-port processors and hierarchy-local
edges remain unported. A hierarchy-local edge now runs recursively through a
child arena, applies its 20px nested rank default, pads the parent, and emits
the inner section in the parent-local frame while the outer edge uses the
resized compound boundary. The JSON entrypoint also reads the adapter's
`spacing.nodeNodeBetweenLayers` and `spacing.edgeNodeBetweenLayers` keys so
qualification cannot silently discard graph-level spacing options.
The child-to-root external-port witness now promotes the owner for root
layering but routes from the actual child port translated into the root frame,
reproducing the source's 5px hierarchical rank offset. Reverse-direction
output is covered; mixed-depth external-port cases remain open.

The source-to-Rust ledger for this slice is tracked in
[`docs/elk-source-structure.json`](elk-source-structure.json). It records the
upstream file, Rust symbol, license/module provenance, focused tests, and the
remaining differences so a bounded phase cannot be mistaken for complete ELK
compatibility.

The current Dagre slice compares `d2layouts/d2dagrelayout/layout.go` directly:
its rank separation is `max(100, maxLabel+40)`, and compound endpoints are
resolved through the longest direct-child chain before routing. The Rust
adapter now follows those rules, including source-arrow reversal and clipping
back to the authored container border. For ordinary graphs, the coordinate
pass also mirrors Dagro's doubled minimum-edge ranks with synthetic zero-size
dummy layers, edge-versus-node separation, stable two-pass barycenter ordering,
and four-way Brandes–Köpf compaction. The minimized cross, reverse-cross,
three-way, join, and split-join witnesses now match the Go 1.27.0 v0.9.0
adapter's node boxes.

The compound pass is now source-shaped for measured graphs. Rust constructs the
border envelope before rank spacing, preserves child coordinates while
`fitPadding` trims each vessel, derives outside-title margins from measured
label dimensions, and emits the 50px Dagro route channel before compacting the
path to cubic points. Single-vessel outgoing/incoming cases match the Go
adapter's boxes and routes exactly in all four flow directions. The two-level
downward nested witness now matches exactly; all fixtures and expected values
are recorded in
`docs/dagre-compound-witnesses.json`. The two-level witness with both
vessel-level and descendant-level crossings now reconstructs Dagro's
per-vessel border records: the inner lane carries `x` and `y`, the outer
sibling lane carries `c` and `w`, and the side lane carries `z` with the outer
vessel expanded before `fitPadding`. The direct `Graph` fixture path
intentionally retains its legacy fitting contract while those source cases are
translated.

The same source-mode rank boundary now accounts for edge-label dummy spans in
the expanded Brandes–Köpf layers. On the labeled crossing witness (`hello` and
`longer label`), Rust matches Go's node order and extra 21px flow rank lane,
with the same stable left-to-right ordering and final node boxes. Dagro's
label-aware compaction is now ported, including direct-value coordinate
alignment and backward block constraints. The normalized dummy route model
also matches the published cubic points after applying d2graph's rounded-delta
rectangle clipping; constrained and compound route channels remain separate
translation slices. A long-edge witness now selects the same network-simplex
rank exchange as Dagro, and the parallel-label witness matches its 20px
label-width gap, label-dummy translation, two-stage rectangle/label clipping,
and outside-node-label endpoint spacing. Unlabeled long edges retain the full
dummy chain as well. The default DFS cycle breaker also follows Dagro's
source-first walk and outgoing insertion order.

The ordinary Dagro ranking and ordering boundary is now qualified through the
source-shaped `acyclic.go`, `network_simplex.go`, and `position_bk.go` paths.
Rust adds the synthetic nesting root for disconnected top-level leaves,
reproduces Dagro's recursive low/lim counter (which intentionally assigns the
same low value to a node and its first descendant), and applies its
feedback-parallel tie rule. The dense feedback witness in
`docs/dagre-source-witnesses.json` and a 5,000-case four-direction randomized
run both match the Go 1.27.0 adapter exactly. The next static comparison is
scoped to Dagro's remaining compound border records and route-channel
construction.

That comparison reached `parent_dummy_chains.go`. Its ownership walk is now
implemented as a diagnostic-only source-shaped probe: it reconstructs the
source/target ancestor path, uses the synthetic border ranks as `minRank` and
`maxRank`, and applies the same ascending/descending rank tests before
assigning a corridor dummy to a vessel. The probe is intentionally gated by
`diagnostic-traces` and `DAGRE_TRY_EXACT_PARENT_DUMMIES`; it is not enabled in
release layouts yet. On a minimized three-vessel crossing witness the probe
changes the child and corridor order away from the pinned Go v0.9.0 SVG, while
the existing ownership heuristics retain the source ordering. This rejects the
probe as a drop-in replacement and leaves the exact missing seam narrowed to
the difference between Rust's normalized chain/border ranks and Dagro's
post-`removeEmptyRanks` temporary graph. The rejected experiment is kept only
for trace/debug follow-up, with no ordinary-build cost.

A separate one-way descendant-crossing probe (`DAGRE_TRY_ONE_WAY_COMPOUND`)
confirms that the bidirectional source-compound gate is only part of the
remaining seam. Enabling the probe widens the normalized border path, but the
minimized witness still differs from Go: the outer vessel is 193px wide versus
222px in the oracle, its child crossing lanes remain 29px too narrow, and its
vertical envelope is 20px taller. The missing behavior is therefore in the
post-ranking border-channel and fit interaction, rather than the gate alone.
This probe is also diagnostic-only and is not enabled for release layouts.

The follow-up diagnostic combines `DAGRE_TRY_SOURCE_FLOW_ORIGIN`,
`DAGRE_TRY_SOURCE_NO_CROSS_SHIFT`, and `DAGRE_TRY_SOURCE_BORDER_WIDTH`. It
seeds the vessel at Dagro's first border rank, keeps its asymmetric BK lanes,
and preserves the rounded border span. With the internal-chain correction
removed from the normal source path, the minimized witness reaches the Go
geometry before renderer quantization: the child boxes and 624px vessel
height are exact, the vessel is 222px wide, and the remaining 0.5px is the
known Rust SVG rounding boundary. A 30-case randomized one-way sweep still
matches only 2/30 cases, so these probes identify the seam without being a
general parity fix. Internal long-edge chains now stay on their BK corridor in
ordinary builds; only the border-origin and one-way gate remain opt-in.

The nested witness trace also isolated one earlier flow mismatch: Dagro's
nesting edges keep a direct leaf in the same normalized rank as a matching
descendant leaf. `prepare_compound_prefit` now applies that rank relation before
measuring the outer vessel. The nested border pass then preserves the source
ordering and side-channel route, with exact output covered by the permanent
two-level fixture.

The pinned `d2layouts/d2elklayout/layout.go` comparison also exposed that
`SpacingOpt` is a transfer concern, not merely renderer metadata. Go widens
each ELK node by its resolved outside-label/icon margins, then moves the
published object by the full top/left band, subtracts all four bands from its
shape box, and shifts descendants by half the opened band. The Rust
source-shaped codec now follows that boundary using `Node::layout_margins`; a
focused parent/child witness checks wire dimensions, published dimensions,
and both coordinate frames. Route endpoint adjustment on the shrinking sides
and the production ELK solver remain separate pending slices.

The pinned `d2renderers/d2raster` comparison found two representation details
that affect exact pixels. Go submits every scanline coordinate through a
float32 reference point and removes expired active edges by swapping with the
last entry; Rust now follows both rules, with the diagnostic path absent from
release builds. D2's ellipse primitive is flattened from f64 scene controls,
so the Rust SVG path is reconstructed analytically before device-edge
rounding. The diamond node witness also required reproducing `lib/shape`'s
source path quantization rather than emitting a generic polygon. The permanent
`raster-diamond`, geometry, and isolated marker witnesses are exact. Text now
selects the bundled Bold face and follows Go's quantized source-space
quadratic flattening; remaining labelled-edge differences are confined to
glyph antialiasing. The tracked witness is `tests/d2/raster-text-bold.d2`,
with the current Go 1.27.0 comparison in
`docs/render-label-edge-go1.27.json`.
The renderer also now treats an explicit `shape: none` arrowhead as
`NoArrowhead` for route shortening; the isolated no-marker route matches the
v0.9.0 PNG witness exactly.

The graph-size comparison also caught a text-shape floor mismatch in the
Markdown path. D2's `d2graph.MIN_SHAPE_SIZE` is 5px; Rust had reused the 12px
floor reserved for class and SQL-table envelopes, widening short text labels
before layout. The compiler now keeps the 5px source minimum, with a one-
character Markdown witness matching the oracle's 9×24 inner SVG.

Remaining high-risk static areas are the full `elk-go` and Dagro algorithms,
Goldmark integration beyond the qualified inline/block helpers, MathJax and
rough.js rendering, raster/text metrics, and multi-board export/navigation.
Those surfaces remain explicitly unfinished in `PLAN.md` and `STATUS.md`.

The latest pinned `elk-go` importer comparison covers node-level port
constraints. Rust now carries `elk.portConstraints` through the arena and
does not replace the supplied cross-axis coordinate for `FIXED_POS` ports;
the side-axis boundary is still normalized before routing. This preserves the
coordinate contract used by D2 SQL-table ports.

That preprocessor migration is now source-shaped through the first transfer
boundary. A deep edge allocates ordered boundary dummies and active per-scope
segments, deactivates the original intermediate edge, transfers the laid-out
dummy position back to the parent port with the Go border offset, resizes the
ancestor vessel around that port, and reconstructs the published route from
the segments. Minimized three-level witnesses now match the pinned Go v0.9.0
geometry and two-bend routes for both directions, including ancestor edges
that reuse an original endpoint. Shared-source fan-out, shared-target fan-in,
and mixed ancestor-source fan-out now also follow the external component
compactor's vertical sibling placement, implicit-port order, vessel sizing,
and outside lane. The DOWN transpose now has matching single-edge, fan-out,
and fan-in witnesses;
Remaining nested-direction cases plus the remaining compactor interactions
remain open.

The CLI boundary comparison also found that v0.9.0's `D2_BUNDLE` switch was
missing. Native rendering now embeds local assets by default and preserves
external hrefs for `--bundle=false`. The static-export appendix is likewise
source-shaped at the semantic SVG boundary: `--force-appendix` adds numbered
link/tooltip entries after rendering, so raster and document exports share the
same deterministic post-layout behavior.

The same SVG comparison caught a Go float32 boundary in curved route control
points. Rust now applies that conversion only while serializing cubic controls;
ordinary layout and orthogonal coordinates remain f64, preserving the source
geometry while matching the v0.9.0 decimal representation used by the raster
oracle.

The export boundary separately keeps `D2_BUNDLE=false` scoped to interactive
SVG packaging. Static PNG/GIF/PDF/PPTX paths still resolve local assets before
rasterization, as the v0.9.0 scene builder does; the CLI regression compares
bundled and explicitly external SVG settings and requires identical raster
bytes.

The nested-layout comparison then found a concrete orchestration gap outside
Dagro itself. D2's `LayoutNested` extracts explicit child-direction scopes and
grid descendants before the parent Dagre pass. Rust was laying the entire arena
at once, so the parent ranker saw the descendants and produced the wrong scope
sizes and disconnected-component placement. The source-shaped fix now reflows
directed scopes with the scope direction, applies nested grids deepest-first,
collapses those descendants before the parent Dagre pass, and restores their
frames and routes by the extracted-scope delta. The pinned
`nested-direction-grid.d2` witness now matches all 24 published rectangles and
the `0 0 1097 1467` viewBox exactly. Raster pixels and cross-scope route
serialization remain open; the measured result is recorded in
`docs/dagre-nested-direction-parity.json`.

The route follow-up closed the serialization half of that witness. The grid
pass now translates descendant routes with their moved subtree, leaves direct
grid-child edges to the grid-owned clipped-line path, and clips cross-scope
fallback endpoints after the parent projection. All 14 published SVG
connection paths now match the Go 1.27.0 oracle; native raster appearance is
still tracked as a separate residual.
