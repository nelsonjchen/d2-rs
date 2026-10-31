<!-- This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
file, You can obtain one at https://mozilla.org/MPL/2.0/. -->

Turn belongs to an original Go `*layoutgraph.Graph`, rather than to a geometry
snapshot or to the first cache entry. A native lazy turn cache needs explicit
same-owner views, independent graph copies and raw cost rollback. This report
is source-only; no native changes, builds or execution support parity claims.

The source manifest freezes 51 relevant original-Go, native-at-read and pinned
Go1.27.0 math files under `.reference/tala-turn-cache-ownership-20261002/`.
Every original-Go file matches its Git object at
`d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda`; the pristine checkout remains clean.
The manifest and complete consumer inventory are linked by the companion JSON.

| Original Go contract | Exact source | Native action required for a shared cache |
|---|---|---|
| Geometry candidate transactions retain the same `Graph` pointer. Ordinary rollback restores geometry/topology, retaining memoized costs. | `layoutgraph/transaction.go:1420` CloneGeometryContext, `:1352` Rollback; `:367` explains selective cost rollback. | Candidate `ArenaGraph::clone` views share the owner. Do not detach ordinary gap, cluster, optimization or binpack trial clones. A first reader on candidate geometry must remain cached after ordinary rejection. |
| A fresh graph begins with zero costs; CopyEntitiesFrom copies grouping/topology maps but no costs. | `layoutgraph/graph.go:763,783` NewGraph→CopyEntitiesFrom, `:1074` CopyEntitiesFrom. | `ArenaGraph::from_input` and `induced_placement_subgraph` construct fresh owner caches; `layoutgraph/arena.rs:542,2124` are the current turn initialization sites. |
| Independent Clone copies raw turn while holding the source lock, creates an independent graph and resets derived EdgeLength cache. | `layoutgraph/clone.go:20` Clone, `:193–202` copy tail. | Add an explicit independent-copy operation that creates a new turn owner initialized from raw bits. Never call a semantic getter while cloning. |
| Primary routing creates fresh split graph owners and then warms their costs. | `routing/graph_stage.go:108,176–177`; `routing/coordinator.go:480`. | `routing/search.rs:5206` clone models a fresh subgraph: detach to a new zero cache before membership-specific routing initialization. Source caller's turn cache must survive unchanged. |
| Standalone/additional routing uses the caller's existing graph and warms existing caches; it does not create or reset a fresh graph. | `routing/standalone_router.go:105–111`; `routing/coordinator.go:531,620`. | `routing/search.rs:5324` is a routing scorer copy, not proof of a fresh Go owner. Preserve caller raw turn and warm the same owner at the original entry; an isolated scorer may capture an independent already-warmed value. Its present reset/recompute and `initialize_standalone_routing_costs` overwrite are separate source differences, not a fresh-owner contract. |
| Each OVG router captures a fixed f64 from its graph getter before flavor workers begin. | `routing/ovg_edge_router.go:166`; `routing/coordinator.go:480–493,620–633`. | Keep route-search arithmetic on the captured scalar for that router context. Do not make every route hop resample an owner getter or geometry. |
| CompoundCandidate clones the incumbent independently before any placement or route reset. | `engine/compound_candidate.go:33`; `engine/pipeline.go:399` ResetPlacementCosts. | `engine/snapshot.rs:1094` must detach before prepare/apply/reset. Its `reset_compound_route_state` at `:947` must never clear the incumbent cache. Inner candidate clones may then share the independent proposal's owner. |
| Stored graph snapshots are independent clones preserving raw costs. | `engine/pipeline.go:53` newRoutingSnapshot→layoutgraph.Clone. | `engine/pipeline.rs:10393` retained graph in LayoutSnapshot must copy an independent raw owner. Historical snapshots must not observe future resets/population on the live owner. Retained diagnostic before/after graph copies follow the same raw-freeze requirement. |
| Explicit placement-cost snapshots restore raw costs into the same original owner; ordinary transaction rollback ignores them. | `layoutgraph/placement_cost.go:52,70`; `layoutgraph/transaction.go:371,397`. | Store raw turn bits in the selective cost rollback state, restoring into the existing owner only at explicit stage failure boundaries. Sharing an Arc with a geometry snapshot does not preserve its prior numeric value. |
| Whole routing stages capture and restore raw costs on error/panic; straight fallback has its own raw rollback. | `routing/route_stage_guard.go:156,464,525`; `routing/coordinator.go:1432,1455`. | Checked native stage snapshots, e.g. `routing/straight.rs:196`, need explicit raw-cost capture/restore. `*graph = snapshot` alone ceases to restore a changed shared cache. Preserve original owner identity when restoring saved bits. |

The primary/additional routing distinction refines the earlier private design
note. Detaching an additional-route scorer for isolation is permissible only
after preserving and correctly warming the caller-owned cache; zeroing it is
not justified by original SplitSubgraphs, which that API does not use.

The complete source inventory has 23 non-test `Graph.TurnCost()` call sites:

- `placement/node_placement.go:355` explicitly warms the owner, then
  HalveTurnCost at357 divides the cached value under the owner lock.
- `placement/sized_optimizer.go:992` reads only in the obstacle-overlap branch.
- `placementcost/edge_length.go` evaluateNodeEdgeLength has13 calls at
  520,532,538,953,956,959,962,966,971,1035,1107,1253,1264. They are conditional
  diagonal, alignment, passthrough, cluster and overlap penalties, reading
  `node.Graph` rather than a replacement node's graph.
- `placementcost/flow_continuity.go:102` reads after its early eligibility
  returns, even when the final continuity coefficient is zero.
- `routing/coordinator.go:480,620` warms the relevant graph before flavors;
  `routing/ovg_edge_router.go:166` captures the warmed scalar.
- `routing/slingshot.go:536,562` and
  `routing/standalone_route_guard.go:241,278` read for positioned arrowhead
  overlap costs.

The exported wrapper `layoutgraph/placement_access.go:23` is the only caller
of internal turnCostValue. Router fields named turnCost are captured scalars,
not additional lazy Graph consumers. Native diagnostics, EdgeLengthState,
serialized graph fields, snapshots and rollback capture must use raw reads.
In particular `placementcost/graph.go:116` takes RoutingCosts before scoring;
native `placement/transactions.rs:7683` currently substitutes turn according
to EdgeLength cache emptiness, which fails the separately verified
NormalizeGaps populated-cache/raw-zero witness.

`Graph.turnCostValue` at `layoutgraph/graph.go:532` checks numeric `!=0.0` under
read lock, then repeats that check under write lock before computing
`(0.125 * float64(len(g.Edges))) * g.maxEdgeLength()`. Both positive and negative
zero remain uninitialized. A computed zero remains the sentinel and is
computed again on the next semantic read. Negative finite values, cached NaN
and either infinity compare unequal to zero and are returned as stored;
there is no finite-value check or sanitization in this getter. Raw snapshots
and independent clones preserve stored bits. ResetTurnCost at621 stores
positive zero; HalveTurnCost at placement_access.go24 divides the raw cache
without forcing initialization. A boolean initialized flag or a bits!=0 test
would change these contracts.

The maximum uses live Graph.Nodes→Node.Edges, including incident edges absent
from Graph.Edges, while the multiplier is exactly len(Graph.Edges). A getter
translation must not interchange those inventories. Geometry NaN sampling is
separate from cached-NaN behavior: original maxEdgeLength uses Go math.Max,
and pinned ARM64 math/dim_arm64.s checks +Inf first then FMAXD. Native
positioned_max_length currently uses `distance > max_length`, so a NaN
candidate distance does not replace its maximum. This is a source observation
requiring its own admissible witness, not part of the finite fixed-node
NormalizeGaps component or evidence to broaden that fix.

Implementable turn-only operations are `raw_turn_cost`, `turn_cost_value`,
`reset_turn_cost`, `set_raw_turn_cost`, `clone_independent_turn_owner(raw_bits)`
and selective `restore_raw_turn_cost(saved_bits)`. Same-owner ordinary clones
share; fresh split owners start at zero; independent graph/snapshot copies
detach with raw bits. Keep crossing and non-center-port ownership unchanged
in this component. Carrier's concurrent representation review covers the
lock/atomic choice; this audit does not select or install one.
