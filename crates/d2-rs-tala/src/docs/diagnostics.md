# Diagnostics and intermediate snapshots

Normal integrations should use [`crate::Engine`] and [`crate::LayoutReport`].
TALA also exposes doc-hidden snapshot types for compatibility analysis. These types describe
implementation stages and are not a stable application API.

```text
input Graph + seed + requested LayoutStage
                 │
                 ▼
       run production stages up to boundary
                 │
                 ▼
            LayoutSnapshot
       ├── node rectangles and cell positions
       ├── edge points and labels
       ├── hierarchy assignment decisions
       ├── hierarchy order/alignment traces
       └── next random-stream probe
```

## Choosing a boundary

[`crate::diagnostic::LayoutStage`] names externally useful observation points, including
sizeless initialization and annealing, the sized transition, hierarchy
placement, deterministic refinement, each routing cleanup stage, labels, and
normalization. A snapshot records the requested stage even when the diagnostic
driver must replay a nearby production boundary to construct it.

## What snapshots are for

- Compare Rust state with a reference implementation at the same stage.
- Determine whether a difference begins in recognition, placement, routing, or
  finishing.
- Inspect hierarchy level order and alignment blocks.
- Verify random-stream consumption without changing the production pipeline.

Do not use snapshots as the input to a later layout call. They are observations,
not resumable checkpoints.

## Diagnostic traces

The opt-in `diagnostic-traces` feature retains environment-controlled trace
probes used during compatibility work. Normal builds compile those probes out.

Enable the feature only for an oracle comparison or a focused debugging build:

```sh
cargo build --profile diagnostic -p d2-rs-lib --features diagnostic-traces --example tala-oracle
```

For pinned v0.9.0 witnesses, `scripts/capture-layout-trace.py` stores raw
stderr and verifies that tracing leaves the public layout bytes unchanged.
The qualified witnesses are `near`, `routing`, `nested-lock`,
`fixed-container`, and `grid-routing`, with additional seed-3/seed-7 runs.
Compare the retained JSON events with:

```sh
uv run --no-project python scripts/compare-layout-trace.py \
  target/parity-witnesses/near.seed-1.trace/go.stderr \
  target/parity-witnesses/near.seed-1.trace/rust.stderr
```

Use `--event stage` or `--event placement` to isolate a family. The comparator
preserves event order, node identities, candidate order, exact IEEE-754 bit
strings, RNG draw counts, placement candidates, scores, and packing events; it
only drops Rust's additive stage `seed` field because the Go v0.9.0 event has
no corresponding field.

The v0.9.0 optimization work guard is also available to diagnostic callers.
`SizelessOptimizer::optimize_with_limit` uses the guarded shuffle and the same
candidate/scoring implementation as the normal pass, so cancellation and
resource-limit witnesses can be compared without putting tracing or error
construction in the release hot path. A successful guarded pass is required to
produce the same geometry and next RNG value as the ordinary pass.

### Routing search

Capture one routing worker's actual heap-pop sequence with
`--search-flavor ShortestToLongest` (also `LongestToShortest`, `Default`, or
`TopDownLeftRight`) on `scripts/capture-layout-trace.py`. Compare those raw
captures with `scripts/compare-layout-trace.py --event route-search-pop`.
Each event includes the edge identity, point coordinates, cost as exact
IEEE-754 bits, and incoming axis. No point sorting or event reordering is
applied. Flavor selection isolates one sequential worker; it does not establish
cross-worker scheduling parity. Quick routes may bypass heap search, so an
empty event family is insufficient evidence. Cost decisions, relaxations, and decrease-key operations still need their own
observations.

Neighbor visits are observed before predecessor, port-direction, and cost
filters. Use `scripts/check-layout-parity.py --search-flavor ShortestToLongest
--search-event route-search-neighbor` to qualify their actual traversal order.
The source-center launch branch is observed as well as ordinary OVG points.

The matrix runner defaults to comparing all `route-search-*` families together
when `--search-flavor` is set. Use `--search-event` only for localization;
family-by-family equality alone cannot establish their relative ordering.

`route-search-cost` records each candidate that reaches the directional
relaxation check: exact candidate/prior cost bits and the selected
push/decrease/reject decision. Filtered-out neighbors do not have cost events.
These observations describe the decision before the update; they do not
establish heap mutation atomicity, displaced-predecessor pushes, or cancellation
behavior. Keep them interleaved with pops and visits for qualification.

Use `capture-layout-trace.py --search-costs` to add the
`route-search-cost-breakdown` family. Each event retains cumulative exact
float bits after the geometric base, structural container/owner penalties,
route interaction, label obstacles, arrowhead labels, turn penalties, and the
final near-object adjustment. This exposes operation-order differences that a
matching total candidate cost can hide; it is diagnostic evidence and is
compiled out of ordinary builds.

Use `capture-layout-trace.py --ovg` to localize visibility construction
mismatches. The opt-in `ovg-topology` records contain scope identities,
obstacle boxes, vertices, and actual adjacency lists. Coordinates use JSON
numbers with round-trip precision, and vertex/neighbor order is retained.
Box-map iteration differs between implementations; these raw records are
localization evidence, not an assertion of normalized topology parity.

`--search-filters` retains Rust's pre-filter state in non-event diagnostic
records: actual predecessor/index, endpoint indices, candidate port metadata,
and container identity, plus route-interaction overlap identities, accepted
route points, and opposing/tunnel/nearby prohibition predicates. These records aid localization and are not included
in parity comparisons. The diagnostic profile keeps release optimization and
symbols, with LTO disabled and 16 codegen units for faster iteration. Capture
manifests record both Rust profile tables and the enabled trace switches.

`TALA_TRACE_RAW_STAGE=1` alongside `TALA_TRACE_JSONL=1` records the raw
arena position, hierarchy membership, scope and active status before the
existing stage projection. These diagnostic records help distinguish actual
algorithm divergence from projection bugs; they are not normalized parity
events. Construction is compiled out without `diagnostic-traces`.
