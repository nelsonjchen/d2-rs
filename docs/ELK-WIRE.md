# ELK wire follow-up at 25e67b4c

This area targets the differential mismatches for authored shape layer
constraints and the Mr. Tree route contract from `f3da1baea`. No oracle
artifacts or scoreboard results were changed or regenerated.

`e1922797b`'s `d2layouts/d2elklayout/layout.go` assigns the uppercase shape
constraint to every leaf/container's `elk.layered.layering.layerConstraint`.
The Rust wire adapter now does that. D2-input tests check the emitted property
and first/last layer placement. The compiler's inherited-scalar cache also
needed disjoint root/style slots: root `elk-layer-constraint` occupied slot 46,
which was already used for `style.opacity` (26 + 20). Its cache regression
checks both lookups against the original public IR lookup.

## Layered wrapping

Source comparison uses elk-go
`v0.2.1-0.20261119175052-8b285820520e`. A local public `elk.LayoutJSON` probe
accepted each option below and returned 12 nodes and 12 routed edges. The
input was a chain of twelve 60x40 nodes, plus a return edge from the last node
to the first, direction RIGHT, algorithm layered, model-order strategy
NODES_AND_EDGES, cycle breaking GREEDY_MODEL_ORDER, thoroughness 8. Each
option was enabled separately; other settings used provider defaults.

| Option | Go result dimensions | Native implementation |
| --- | --- | --- |
| `elk.layered.wrapping.strategy=SINGLE_EDGE` | 964x75 | Ported: graph statistics, MSD/ARD/manual cuts, greedy/look-back validation, layer folding and return-route dummy chains before node placement. |
| `elk.layered.wrapping.strategy=MULTI_EDGE` | 394x258 | Ported for graphs without center edge labels: paired breaking-point insertion, cut improvement, folding and chain shortening, and orthogonal edge restoration. Center labels remain explicitly unsupported. |

SINGLE_EDGE now runs in root/component and nested scopes before node sizing
and placement. Wrapping dummies retain individual edge/node spacing,
OriginalElement edge identity, fixed ports, thickness and long-edge endpoints;
the existing join/reversal postpasses restore the original edges. Contract
tests port `TestSingleEdgeManualCutWrapsPath`, verify cut heuristics and the
source's explicit-forbidden-indices behavior, and exercise D2 paths in all
four directions. This targets the SINGLE_EDGE differential mismatch cluster;
no dimension-only special case or oracle regeneration was used.

MULTI_EDGE now inserts paired breaking-point layers before LongEdgeSplitter
and node ordering; folds chunks before placement (and before the flat greedy
switch processors); and rejoins the original edges after long-edge joining,
before north/south port restoration. The port includes distance-penalized cut
improvement, linked multi-cut metadata, both dummy-chain shortening passes,
and bend/junction restoration. Detached chains are excluded from Rust's
active-edge scans, matching Go's traversal through layered nodes.

Tests port `TestMultiEdgeInserterCreatesPairedBreakingPoints` and
`TestMultiEdgeManualWithoutCutsDoesNotFallThroughToARD`; a two-cut spanning
edge checks original endpoint and edge identities with chain improvement
both enabled and disabled. D2-input paths cover both strategies and all
four directions. The documented 12-node loop is now also a Rust contract:
both dimensions above match, with 12 nodes and 12 complete orthogonal routes.
Nested cycles exercise both strategies in all four directions.

**Remaining gap:** MULTI_EDGE combined with center edge labels is rejected
with `elk-wrappingStrategy=MULTI_EDGE: center-label edge restoration is not
yet ported`. Native label switching can retire the `NodeStartEdge` segment
recorded in breaking-point metadata before the remover uses it. Faithful
interaction with the label processors still needs source comparison and
qualification; the native path must not panic or silently ignore wrapping.
The nested-cycle contract checks this explicit error for MULTI_EDGE and
checks successful labeled routing for SINGLE_EDGE in all four directions.
Spline-specific compaction-segment restoration from the upstream unit test
is not ported: this D2 layered pipeline uses the orthogonal router.

These changes target the SINGLE_EDGE and MULTI_EDGE differential mismatch
clusters. No oracle artifacts, fixture regeneration or broad scoreboards
were run. Validation: `cargo build -p d2-rs-elk` and
`cargo test -p d2-rs-elk` (183 unit and 8 integration tests passed, one
pre-existing ignored unit test, zero doc tests).

## Forced model order

`elk.layered.crossingMinimization.forceNodeModelOrder=true` now selects the
elk-go `8b285820520e` model-order barycenter comparator for boundary and free
layers on every sweep. The importer retains node model order separately from
mutable traversal indices, honors `noModelOrder`, and records group IDs and
the maximum order used by enforced group comparisons. The comparator mirrors
insertion sorting, transitive comparison history (cleared per layer),
group-aware comparisons and barycenter fallback for equal/absent model order.
Forced sorts bypass Forster constraint resolution as upstream does.

The D2-input contract preserves authored order despite opposing crossed-edge
barycenters in all four directions. A native comparator contract also checks
configured groups, the both-groups-enforced rule, and equal/absent-order
fallbacks. This targets the forced-model-order differential mismatch cluster;
no scoreboard or oracle fixture regeneration was run.

## Feedback edges

`elk.layered.feedbackEdges=true` now schedules `PortSideProcessor` before P1
(after the edge/layer-constraint reverser, following the source enum order),
instead of before P3. Both root/component and nested scopes preserve those
fixed sides through cycle reversal and run inverted-port processing even for
originally free ports. The existing north/south pre/postprocessors remain
conditional on connected fixed north/south ports; no unconditional dummy
insertion was added. Collector adaptation remains the upstream reversal path.

The D2-input cycle contract checks all three edges, including the feedback
edge, for forward source/target approaches and orthogonal routing in all four
directions, in both flat and container graphs. This targets the feedback-edge differential mismatch cluster.

## Mr. Tree

`f3da1baea` changes the adapter, not the tree-routing algorithm: use the
provider's root edge-node clearance, preserve root tree detours during bend
cleanup, and trim repeated bends and bends inside endpoint shapes before
shape tracing. Those changes were already present in the preceding port.

The new `mr_tree_routes_avoid_shapes` contract ports the upstream measured
input (`Root -> API -> Cache -> Store`, `Root -> Store`, `Root -> Web`), all
four directions and DFS/BFS. It checks attachment approach, repeated points
and every segment against every shape with the upstream one-pixel inset.
It passes without another routing change.

The previous hand-sized endpoint test does not establish general avoidance
for arbitrary geometry. In particular, `mrtree::routing::Arena::skip_gap`
matches Go's `shouldSkipMultiLevelGap`, including the intentional `first.X`
comparison in the horizontal branch. Replacing it with a general obstacle
router would depart from the target source. No universal collision-free
guarantee is claimed beyond the measured upstream contract.
