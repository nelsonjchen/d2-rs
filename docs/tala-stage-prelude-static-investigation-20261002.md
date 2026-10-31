<!-- This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
file, You can obtain one at https://mozilla.org/MPL/2.0/. -->

# TALA engine prelude investigation

Static inspection found two unconditional cell-size computations present in original D2 v0.9.0 and absent from the current native stage wrappers: `OptimizeClusters` and `Equidistance`. The graph helpers are different contracts: Go `placement.OptimizeClusters` and `placement.Equidistance` do not own these computations. A fix belongs at the engine wrapper, without moving trial-admission or completed-stage dimension checks.

The frozen original-Go prepared engine test already records cell 10 (`0x4024000000000000`) after the real `optimizeClustersStage` in all three 29999/30000/30001 width controls. It starts with cell 20 and invokes actual `AddClusters` after the real stage-zero reset. Current native prepared tests execute the graph helper directly with cell 20 and eagerly initialized scoring costs. Those tests establish admission and geometry behavior; they do not establish the actual wrapper prelude or operation trace. No engine was run for this audit.

The smallest next paired wrapper control should derive prior cell 20 from the real cell formula, rather than assigning it: keep the existing five-node graph but begin the independent blocker at width 13, compute its prior cell, then shrink the blocker to width 1 in the preparation stage before actual automatic grouping. With active row vessel 220x101, original wrapper computation derives cell 10. Share the native production wrapper with the prepared test, compare raw cell/cache state before the first actual score, and keep existing direct-helper admission tests separate. Extracting the native wrapper is sufficient to expose this actual contract without replaying preceding unrelated placement stages. Preserve the real final dimension gate and all prior failing captures.

A separate successful automatic Row-to-Column flip can exercise Equidistance's recomputation: two 100x101 members and padding 20 produce a 220x101 row vessel and a 100x222 column vessel. With a 100x100 external peer, static cell arithmetic changes 220 to 222. The actual flip and its runtime outcomes still require a fresh original-Go/native witness; arithmetic alone cannot qualify an algorithm result.

Further prelude candidates need observation before fixes. Native recursive NodePlacement computes cell size before its final vessel-size refresh; original Swap and Transpose recompute unconditionally before their work. A later call can be redundant on unchanged graph dimensions, so freeze live node order and exact active size bits to determine applicability. Original NormalizeGaps resets only the lazy turn cache before SyncSequences and SyncClusters; native publishes those aggregates first and eagerly recomputes turn cost. Observe raw cache values and first materialization, not getters that would populate the cache as a side effect. Do not fold these separate concerns into the small wrapper change.

The companion JSON stores 22 source snapshots, original-Go capture hashes, exact line excerpts, deferred test design and all 549 current compiled native source checks. No Rust or Go implementation was edited, no source oracle was rebuilt, and no new parity gate is claimed.
