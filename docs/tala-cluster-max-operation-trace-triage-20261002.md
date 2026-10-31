# Current operation trace qualification: failed gates retained

The frozen current oracle pair builds successfully. The five operation capture
reports remain **FAIL**: placement qualifies eight emitted-scope cases and lacks
required evidence in eight; the three ordinary routing flavors each have twelve
schema divergences and four cases without search work; `TopDownLeftRight` lacks
search work in all sixteen cases. This audit changes no comparison predicate,
presence gate, event order, geometry, implementation, or observer driver.

The independent [raw audit report](/Users/nelson/code/d2-rs/.reference/tala-cluster-max-operation-raw-audit-20261002-retry-1/report.json)
reparses all 80 cases, reproduces each verdict and verifies 4,985 distinct
artifact records before/after reading. The original source checkout is pristine
at `d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda`. The audit ran no engine or
compiler. Public measured-graph output, same-side tracing noninterference,
diagnostic/release equivalence and selected-stream repeatability hold in every
captured case. These are bounded observations, not full D2 or TALA parity.

| Capture | Qualified emitted scope | Divergent | Insufficient | Selected events per side |
|---|---:|---:|---:|---:|
| Placement, all emitted families | 8 | 0 | 8 | 60,566 |
| `ShortestToLongest` search | 0 | 12 | 4 | 27,148 |
| `LongestToShortest` search | 0 | 12 | 4 | 27,148 |
| `Default` search | 0 | 12 | 4 | 27,148 |
| `TopDownLeftRight` search | 0 | 0 | 16 | 0 |

Each capture covers four existing fixtures, seeds 1/42 and two repetitions.
The [machine-readable triage](/Users/nelson/code/d2-rs/docs/tala-cluster-max-operation-trace-triage-20261002.json)
binds complete report/binary/helper hashes, source snapshots and the earlier
failed audit attempt. The Rust example pair is diagnostic
`30d629d3fe6d9634dd66fc49bef8c39988debddd7b3266cdb9bb59d3b620f5d7`
and release
`12d33edc5e7721db2fa1feee7197b343162f108b6887b9cdb194820f636dcdd8`;
the terminal pair manifest is
`8a72fc9a7e899e3e5a1cc83c9b594cc8196e6b887e731872eead0e357a79dd66`.

## Missing placement families have source-supported no-op paths

`routing.d2` explicitly fixes all three objects' left/top coordinates. The
initialized raw snapshot carries exact fixed-coordinate bits for `source`,
`target` and `obstacle`. Both sides retain the same three-node sized worklist,
emit 156 sized-order callbacks and consume 156 RNG events per case, then emit
no candidate or score records. Original Go
[sizeless eligibility](/Users/nelson/code/d2/d2layouts/d2talalayout/internal/placement/sizeless_optimizer.go:598)
filters fixed positions. The
[sized loop](/Users/nelson/code/d2/d2layouts/d2talalayout/internal/placement/sized_optimizer.go:285)
shuffles all graph nodes before skipping fixed nodes at line 304. Native
[eligibility](/Users/nelson/code/d2-rs/crates/d2-tala-rs/src/internal/layoutgraph/arena.rs:976)
and the [sized skip](/Users/nelson/code/d2-rs/crates/d2-tala-rs/src/internal/placement/sized.rs:1878)
support the same explanation. Skipping optimization does not skip shuffle RNG.

`fixed-container.d2` requests dimensions for a container named `fixed`; it
does **not** fix any object's top-left position. The measured input has five
objects, zero edges, no near keys and no left/top constraints. Every recorded
sized scope is a singleton: `fixed.a`, `fixed.b`, `fixed.c`, `fixed.d`, then
`fixed`, each visited by 45 before-shuffle callbacks. Both sides emit 450
before/after callbacks in total, with no primitive RNG draw, candidate or score
events. Original
[child-graph splitting](/Users/nelson/code/d2/d2layouts/d2talalayout/internal/placement/node_placement.go:145)
creates placement subgraphs; both original optimizer guards reject edgeless
nodes without usable nears. Native
[sizeless inventory](/Users/nelson/code/d2-rs/crates/d2-tala-rs/src/internal/layoutgraph/arena.rs:1071),
[sized inventory](/Users/nelson/code/d2-rs/crates/d2-tala-rs/src/internal/layoutgraph/arena.rs:1087)
and [edgeless skip](/Users/nelson/code/d2-rs/crates/d2-tala-rs/src/internal/placement/sized.rs:1884)
support that path. Singleton shuffles need no random draw. The wider owner
snapshot includes contextual descendants; it is not the optimizer worklist.

These witnesses do not expose a missing candidate/score callback on a node
that reached those operations. They **do** expose missing runtime eligibility
evidence: neither captured stream records each constructor rejection or sized
visit's exact skip reason. Static explanation cannot qualify those un-emitted
operations. The eight cases remain insufficient under the unchanged gate.
Original Go explicitly skips `NodeToTree` members; native uses a live graph
inventory with retained tree nodes removed, rather than an identical check in
the shown loop. These two negative controls do not qualify tree ownership,
oversize exclusions, usable-near traversal, or cancellation/work boundaries.

## Search fails at the frozen Go observer schema

In each ordinary search capture the first selected event is a heap push. Its
coordinates, priority bits, queue length, orientation, operation, edge and
flavor match. Native additionally emits `work`, `scope_nodes` and `scope_edges`;
the frozen Go observer omits all three. The supplemental exact comparator
therefore reports `$[0]` with three extra native keys. Across all 5,484 heap
events per flavor these fields are absent on Go; no other schema or shared-value
difference was observed at the original event indices. That inventory is
descriptive only: the full schema comparison remains divergent, and meaningful
native fields were never removed.

The authoritative observer source for that Go binary is the recorded patch
`97b3908731d92cd4a299ca0eb585a440538cfd7832de54353c95aa8198d95fba`,
whose [heap emitter](/Users/nelson/code/d2-rs/tools/patches/d2-v090-stage-trace.patch:5900)
lacks those fields. Native's
[compiled emitter](/Users/nelson/code/d2-rs/crates/d2-tala-rs/src/internal/routing/visibility.rs:3547)
reads the actual local work guard and graph inventories. The current live Go
instrumentation contains later additions; it is not claimed as compiled source
for the frozen binary. A new source-inventoried Go observer build must record
actual work/scope values at the matching operation point, preserve ordinary
results, and receive fresh qualification before these fields can be compared.
Matching existing search values does not establish work-accounting parity.

The four `fixed-container` cases in every ordinary flavor have no edges and no
search or route-flavor callbacks. Their missing search families are a
source-supported no-work witness, not evidence of an eligible search whose
callback disappeared. They retain insufficient status.

`--search-flavor TopDownLeftRight` selects an observer, not the algorithm's
flavor. All twelve cases with routing emit the actual ordinary callback order
`ShortestToLongest`, `LongestToShortest`, `Default`; the remaining four emit no
flavor callbacks. Original Go selects `TopDownLeftRight` only when the first
active routing node carries
[hierarchy state](/Users/nelson/code/d2/d2layouts/d2talalayout/internal/routing/coordinator.go:483);
native checks the
[same carrier condition](/Users/nelson/code/d2-rs/crates/d2-tala-rs/src/internal/routing/search.rs:4475).
A container or fixture name alone does not satisfy it. No current TopDown
search execution, work, or event-schema coverage is claimed.

## Next hooks and witnesses

1. Add diagnostic-only per-node eligibility/visit callbacks on both sides:
   actual scope and ordered stable identities, fixed-position bits, edge/near
   inputs, tree membership, cell/size bits, inspected near order, accepted or
   skipped reason, and work/cancellation point. Record constructor filtering
   separately from post-shuffle sized visits. Preserve empty/no-op controls;
   do not silently exempt them from the current presence gate.
2. Qualify a new Go heap observer with actual scope and work state. Capture
   before/after work charges where needed; do not invent values or copy native
   fields. Re-run the existing ordinary-flavor controls against the same
   frozen Rust pair, then investigate the earliest real value divergence.
3. Add a source-backed hierarchy witness using the topology in original
   [forced-hierarchy test](/Users/nelson/code/d2/d2layouts/d2talalayout/fixed_hierarchy_test.go:11),
   and require the actual `TopDownLeftRight` callback and all five nonempty
   search families. Confirm the hierarchy carrier before counting coverage.
   Add mixed fixed/free cyclic placement and a dimension-constrained container
   with connected movable children; avoid tree or hierarchy extraction turning
   the intended ordinary-placement witness into another no-op.
4. Add real-stage prelude/cache callbacks. Original
   [OptimizeClusters wrapper](/Users/nelson/code/d2/d2layouts/d2talalayout/internal/engine/pipeline.go:202)
   recomputes CellSize immediately; the shown native
   [stage entry](/Users/nelson/code/d2-rs/crates/d2-tala-rs/src/internal/engine/snapshot.rs:619)
   does not explicitly do so there. The separately prepared cluster witnesses
   use cell size 20 on both sides, while the real original wrapper observes 10.
   Emit ordered compute inputs, exact old/new cell bits and cache materialization
   before claiming wrapper-operation parity. This is a separate gate from the
   already qualified cluster trial-admission correction.

Generic score/cache components, complete transactions and rollback events,
actual map/sort traversal, work/cancellation ordering and global worker/seed
scheduling remain open. The original comparator can coalesce signed zero;
the retained supplemental decoder checks represented IEEE-754 numbers and
emitted bit strings. Neither recovers absent arithmetic, geometry or NaN
payloads. Original stage-seed and adapter-local stage edge-ID omissions remain
explicit limitations. Official release use in these wrappers is a hash pin;
these new captures do not provide official CLI SVG/pixel noninterference.

The first independent audit attempt failed while reconstructing the original
empty-stream rejection and was preserved. The fresh retry includes that rule;
it also corrects a descriptive scope-inventory field name. This repair changes
no capture/comparison helper or retained qualification result.
