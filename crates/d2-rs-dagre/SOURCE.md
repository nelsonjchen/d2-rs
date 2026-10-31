# Corresponding source

Run 82 (2026-10-25), master `6a7b48378`: graph/context admission maps
`d2layouts/d2dagrelayout/layout.go:48–63` and the shared layoutguard before
Dagre graph translation/solver work. The existing entry points use the same
check; the cancellation-aware entry points retain `failed to dagre layout`.
The original dense-graph and pre-canceled tests change from success to their
complete source errors. Geometry algorithms are unchanged.

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

Checkpoint 873: outside-label route interception follows pinned
`d2graph/d2graph.go:Object.HasLabel` and `d2graph/layout.go:Edge.TraceToShape`: text,
code, class and SQL intrinsic content does not form an outside-label obstacle.

Checkpoint 874: direct compound/external-edge paths publish the original
`d2dagrelayout/layout.go::adjustCrossRankSpacing` once before `fitPadding`,
without applying the legacy title/icon clearance a second time.

Checkpoint 874 routing: carry direct compound external-edge routes through
spacing, fitPadding and final tracing in every direction, following the pinned
`d2dagrelayout/layout.go` pipeline. Remove the synthetic horizontal bend/lane
override; original Dagro route state supplies the bends.

When the complete cross-spacing pass publishes compound boxes, carry every
internal route as well as the direct container edge; source `edge.Route` shares
that spacing state. Three full source curves are regression contracts.

The raw route coordinate frame is compared with the published boxes before
fitPadding. Equivalent legacy compound box reconstruction retains shared routes
too. An isolated fork/grid direction-root regression follows original Go box
geometry and its horizontal curve (font-size24).

Checkpoint 875: d2layouts/d2dagrelayout/layout.go integer-size Dagro input and rounded published node origins; preserve unrounded measured flat-rank route centers and recover disconnected component translation at the rounded-origin boundary. Regression: odd isolated SQL table beside an ordinary measured edge.

Checkpoint 875 reverse-direction refinement: infer the anchor translation from published rounded origin minus model rounded origin, rather than its published center. Original Go four-direction controls retain twelve boxes and sixteen points, including reverse flow when an odd maximum span differs from the anchor span.
The rounded-origin reconstruction is scoped to measured flat ranks; existing compound frame translations remain unchanged.

Checkpoint 876: Source D2-shaped measured nodes accept zero dimensions as source Dagro does; generic graph inputs retain positive-size validation. Full-box source zero-width cloud regression, no clamping.

Flat long-edge label continuation (2026-10-18): BkModel::from_graph now
translates dagro0.2.1 layout.go::injectEdgeLabelProxies/removeEdgeLabelProxies
and normalize.go::normalizeEdge consistently for flat ranks. Only the dummy at
the endpoint-rank midpoint carries the label's flow/cross span; selecting the
first intermediate rank displaced nodes and corrupted route reconstruction for
long labeled edges. The existing compound proxy-derived labelRank is retained.
Sixteen independently captured original-Go measured graphs cover all four
orientations, three/four-node chains and ordinary/HandDrawn dimensions. The
permanent flat-long-label-source-v090.json fixture compares all published node
boxes and complete unnormalized graph routes (f64 tolerance1e-9). Exact public
SVG/browser/PNG output remains a separate end-to-end gate. Original public
oracle data and the instrumented worktree's public outputs match for the
initial witness; diagnostics remain compiled out without diagnostic-traces.

## Single-container raw dummy-chain routes (2026-10-22)

`initial_source_routes` uses `DagreRouteModel::source_adapter_points` whenever
its translated border model is present, including a single root container.
Original D2 `d2layouts/d2dagrelayout/layout.go:210–273` reads every Dagro
edge's raw `points`, clips the chain to the authored boxes once, and carries
it through rank spacing, cross spacing and `fitContainerPadding`; root
container count does not select a different route reconstruction. Reduced
from upstream `stable/elk_shim`: `tests/regressions/e2e/
dagre-single-container-dummy-lanes.d2` (complete normalized SVG equals pinned
Go, target hash 2405017235). The CLI regression also checks the target hash.
Full upstream delta: +3 exact SVG boards, zero regressions.

Compound-edge silhouette tracing (2026-10-22 run 4) follows original D2
`d2graph/layout.go:487–499, 575–587` and
`d2layouts/d2dagrelayout/layout.go:348–351`: the final TraceToShape intersects
the rectangle once before tracing the shape perimeter. Removed a second
compound-only rectangle intersection after that pass; rounded displacements
changed the ray by a pixel. Reduced witness:
`tests/regressions/e2e/dagre-compound-shape-trace.d2`; complete SVG matches Go,
target 2161879430 → 2428849042. Full scoreboard +2 / 0 for Dagre
`themes/terminal_grayscale` and `txtar/shape-animate`. Origami and Terminal
remain separate failures; this does not qualify all endpoint geometry.

## Modifier-relative decoration clipping (2026-10-23 run 16)

Original `d2layouts/d2dagrelayout/layout.go:324–343` temporarily moves a
3D/multiple endpoint's box before `d2graph/layout.go:414–438` computes its
outside label/icon geometry. `curve_fitted_routes` now gives those helpers
the same modifier-adjusted published box used for shape clipping. Previously
the label stayed at its unadjusted position. Reduced from failing
`txtar/md-tables`: `tests/regressions/e2e/dagre-modifier-label-trace.d2`,
target 3890689203 → pinned-Go 2721557747, complete normalized SVG equality.
Full upstream delta: +1 exact SVG / zero regressions.

## Published self-loop centers (2026-10-23 run 17)

Original `d2layouts/d2dagrelayout/layout.go:202–204` rounds each object box
before lines 235–240 add its center as both self-loop sentinels. Dagro's
interior controls retain their fractional coordinates. `dagre_self_loop_points`
now uses the published box only for those sentinels, preserving the model
center for the controls. Reduced from `txtar/bidirectional-connection-animation`:
`tests/regressions/e2e/dagre-self-loop-published-center.d2`, target
3482141975 → pinned-Go 1485139401; complete normalized SVG equality.

## Outside-label step-back and fitted child origins (2026-10-23)

- `trace_route_with_decorations_like_d2` now ports the destination-side loop
  of `d2graph/layout.go:516–565` (Edge.TraceToShape): while the ending
  segment's start lies inside the outside label (or, failing that, icon) box,
  the segment steps back and the trailing point is dropped before
  intersecting. The source-side loop's `startIndex+1 > endIndex` guard never
  holds in v0.9.0 and is intentionally not ported. Unit test
  `target_label_step_back_drops_points_inside_the_label` uses the
  `regression/dagre_broken_arrowhead` route; its endpoint now equals the golden.
- `fit_containers_source` keeps an already fitted nested container's fractional
  origin when fitting its parent (layout.go fitPadding writes TopLeft back),
  instead of re-rounding it.
- Resolved below: the remaining one-pixel offset was publication rounding.
- Measured: the source-mode route overrides (`override_flat_reciprocal_routes`,
  `override_vertical_wide_external_routes`, `adjust_opposing_nested_route_bias`,
  `adjust_flat_port_routes`) have no source counterpart; disabling all four
  changes no upstream e2e board (922 before and after).

## Endpoint shapes, mixed external order and fitted origins (2026-10-23)

- `trace_route_with_shapes_like_d2` applies `TraceToShape`'s shape-border
  trace (`d2graph/layout.go`) to each non-rectangular endpoint after its box
  intersection and before the opposite endpoint is clipped, skipping endpoints
  already clipped by decorations. +1 `stable/chaos2` (cylinder source).
- Removed `reorder_mixed_external_anchor_positions`,
  `shift_reordered_external_routes` and `canonical_endpoint_node`: they
  reordered root anchors around a single compound after fitPadding (matching
  endpoints by short id), which `d2dagrelayout` never does; Dagro's order is
  final. +1 `todo/container_label_edge_adjustment`, witness
  `tests/regressions/e2e/dagre-mixed-external-anchor-order.d2` (pinned Go
  4222133879; previous Rust 1879626039).
- `layout.go:202` rounds Dagro boxes, but `fitPadding` writes a fractional
  TopLeft that d2exporter publishes with `int()` (truncation).
  `fit_containers_source_tracked` reports the containers it fitted; source-mode
  publication truncates those and keeps rounding everything else (truncating
  all boxes regressed 7 boards). +1 `regression/dagre_broken_arrowhead`,
  witness `tests/regressions/e2e/dagre-fitted-origin-truncation.d2` (pinned Go
  4180049586; previous Rust 3629425432).
- Full upstream delta: 929 → 933, zero regressions; every applicable Dagre
  board matches.
