<!-- This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
file, You can obtain one at https://mozilla.org/MPL/2.0/. -->

Original Go v0.9.0's actual `pipeline.gapNormalizationStage` returns with its
raw turn cache zero. A later actual `Graph.TurnCost()` reader materializes
112.5 (`0x405c200000000000`) for the two-node, one-edge witness; another reader
returns those same bits. This is a verified Go cache-lifecycle component, not a
native parity claim.

The witness uses two fixed 100×100 nodes at (1000,1000) and (2000,1000), an
ordinary connecting edge and the real cell computation. Source is pristine
commit `d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda`; build and embedded executable
metadata both report Go1.27.0. A virtual overlay adds one raw-state callback
after the actual placement stage returns. It invokes no score getter. All
4,495 original files are separately frozen and reverified; the original
checkout remains clean.

| Control | Entry raw turn | Returned raw turn | Actual first/subsequent reader | Returned EdgeLength entries |
|---|---:|---:|---:|---:|
| Uncached | 0 | 0 | 112.5 / 112.5 | 2 |
| Already cached, current | 112.5 | 0 | 112.5 / 112.5 | 2 |
| Already cached, stale | 100 | 0 | 112.5 / 112.5 | 2 |
| Zero edges | 0 | 0 | 0 / 0 | 0 |

The stale value is produced by a real first reader with the second node at
(1900,1000), followed by moving that node and its fixed position to (2000,1000)
before the actual stage. No cache value or cell is manually assigned. Each
control runs with the raw observer disabled and enabled: all eight complete
results are identical across that pair, including geometry, raw costs, cache
counts, changed flag and errors. Stage geometry and cell are bit-identical to
entry. In the one-edge cases, crossing is materialized by actual stage work as
`0x4058e219652bd3c3`, two EdgeLength entries survive, and only the later explicit
TurnCost getter changes raw turn. The first/subsequent readers leave all other
recorded state unchanged.

Attempt1 is retained as a failed expectation. It incorrectly assumed that the
stage's changed flag would be false for fixed, unchanged geometry. Original
`gapNormalization` sets changed=true after a successful transaction commit,
including no-op commits; the final witness records this flag exactly and
checks geometry independently. Attempt2 removes only that false expectation.

Source order is `ResetTurnCost` → `SyncSequences` → `SyncClusters`. The reset
clears only turn. `RoutingCosts` and graph serialization copy raw fields;
`turnCostValue` samples live geometry only when its cached value is zero.
EdgeLength computes its state key from raw RoutingCosts before any node score
can call TurnCost.

Native static inspection finds a bounded next difference:
`hierarchy.rs` currently syncs aggregates and then eagerly computes turn in
`refresh_turn_cost_after_gap_normalization`; `transactions.rs` treats cache
emptiness as the routing-cost initialization boundary. The original witness
has raw turn0 alongside two cache entries, so that proxy cannot represent turn
after this reset. A native change needs distinct raw and semantic turn reads,
with first-use sampling at the original getter sites, and explicit same-owner
versus independent scope cache ownership. Zeroing the existing scalar alone
would make its current direct semantic consumers score with zero.

The [source-qualified manifest](tala-normalize-gaps-lazy-turn-go-witness-20261002.json)
records complete report, overlay, toolchain and source-proof hashes. Full raw
observations, the failed attempt and the deferred native design are stored in
`.reference/tala-gap-turn-cache-witness-20261002/`. No native implementation
was changed, compiled or executed for this investigation.
