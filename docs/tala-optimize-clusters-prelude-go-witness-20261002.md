<!-- This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
file, You can obtain one at https://mozilla.org/MPL/2.0/. -->

# Original-Go OptimizeClusters prelude witness

Six source-isolated controls passed on pinned original D2 v0.9.0 and Go 1.27.0, with twelve observer off/on executions. The hook runs immediately after the actual engine wrapper computes its cell size and before its first score: every enabled hook observed zero raw scoring caches and an empty edge-length cache. Exact geometry, cell values, raw costs, cache entry counts, changed gate and errors matched across observer modes.

The clustered controls derive prior cell 20 from a real minimum dimension 13, then shrink the independent blocker to width 1 and invoke actual automatic grouping. The actual wrapper recomputes cell 10; no arbitrary cell or cluster arrangement is assigned. Real optimization moves the connected peer by 99/100/101, yielding widths 29999/30000/30001 and height 2100 with selected score bits `0x40bc200000000000`. The stage helper admits the above-limit case; its separate completed-stage dimension gate remains unchanged. Unchanged-cell 20 and already-current-cell 10 controls pass. A graph without clusters derives cell 100 from 100x100 dimensions, shrinks to 13x100 and verifies that the actual wrapper still recomputes cell 20 despite having no cluster work.

A fresh pinned Git archive supplies 4,495 pristine files; the virtual overlay maps Go/module sources from that snapshot, adds only a raw-state hook and a unique test, and leaves the original checkout pristine. Removing the hook restores the exact original pipeline source. The executed test binary embeds Go 1.27.0.

Native source remains unchanged. Private proposed patches extract the existing production wrapper, share it with six prepared cell-contract tests, and then add exactly one unconditional `compute_cell_size` call. Neither proposal has been installed, compiled or run. The expected native BEFORE failures are three stale20-to10 controls and the empty-cluster100-to20 control. Existing direct-helper tests do not establish this wrapper contract. Raw cache/operation parity, full authored NodePlacement, Equidistance and broader D2 qualification remain separate.
