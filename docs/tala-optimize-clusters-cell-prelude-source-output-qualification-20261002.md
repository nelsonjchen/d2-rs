<!-- This Source Code Form is subject to the terms of the Mozilla Public
License, v. 2.0. If a copy of the MPL was not distributed with this
file, You can obtain one at https://mozilla.org/MPL/2.0/. -->

# OptimizeClusters cell-prelude qualification

The native engine now recomputes cell size immediately before cluster optimization, matching original D2 v0.9.0's engine wrapper. The shared wrapper is called by the production snapshot/layout path and six prepared contract tests. Its extraction preserves the surrounding cancellation, trace and completed-stage dimension checks; the graph helper and scoring/cache functions are unchanged.

The native BEFORE run reproduced four failures and two passing controls. Three controls retained cell 20 instead of source cell 10; the graph without clusters retained cell 100 instead of recomputing cell 20. The AFTER build passes all six assertions within 853 TALA tests plus 2 documentation tests, with 25 ignored. Render 106, export 73, library 62 plus 1 documentation test and CLI 161 also pass. These six controls assert the wrapper's cell contract; they do not capture native scoring caches, final geometry or detailed execution traces.

The paired source controls use actual Go 1.27.0 wrapper code and automatic grouping. Prior cells come from real node dimensions instead of arbitrary assignment. Six cases and twelve observer off/on executions passed; every enabled pre-score hook observed zero raw costs and an empty edge-length cache. Exact before/after geometry, costs, cell, cache entry counts, gate and errors matched across observer modes. This raw cache and observer proof applies to the original-Go controls only.

Independent source verification confirms only the pipeline and snapshot files changed among 549 compiled sources, with the other 547 byte-identical to the preceding qualified build. Removing the new wrapper, six-test module and single production-caller replacement restores the preceding source exactly. The extracted native BEFORE and AFTER differ only by one `compute_cell_size()` call. The audit rehashed 2,234 artifact references and 4,495 pristine-Go source files. Earlier reports retain their original capture-time statements; this qualification records the subsequent installation and native runs separately.

Final-output, pixel, RNG, feature-off and measured-oracle operation captures are separate qualifications. No full TALA/D2 parity, authored full-NodePlacement reachability, race or performance claim follows from this component.
